use crate::{
    BusValue, CartridgeConfig, Mapper, MapperError, Mirroring, NametableTarget, ReadMapping,
    ResetKind, WriteMapping,
};

use super::{bank_offset, nametable_target};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Variant {
    Combined,
    Vrc4f,
    Vrc4e,
    Vrc2b,
}

impl Variant {
    fn has_vrc4_features(self) -> bool {
        self != Self::Vrc2b
    }
}

/// Konami VRC2b/VRC4e/VRC4f (iNES mapper 23).
#[derive(Debug)]
pub struct M023 {
    config: CartridgeConfig,
    variant: Variant,
    prg_banks: [u8; 2],
    prg_swap: bool,
    ram_enabled: bool,
    chr_low: [u8; 8],
    chr_high: [u8; 8],
    mirroring: Mirroring,
    microwire_latch: u8,
    irq_latch: u8,
    irq_counter: u8,
    irq_prescaler: i16,
    irq_enabled: bool,
    irq_enable_after_ack: bool,
    irq_cycle_mode: bool,
    irq_pending: bool,
}

impl M023 {
    pub fn new(config: CartridgeConfig) -> Result<Self, MapperError> {
        let variant = match config.submapper {
            0 => Variant::Combined,
            1 => Variant::Vrc4f,
            2 => Variant::Vrc4e,
            3 => Variant::Vrc2b,
            submapper => {
                return Err(MapperError::Unsupported {
                    mapper_id: 23,
                    submapper,
                });
            }
        };
        if config.prg_rom_len < 0x8000
            || config.prg_rom_len > 0x40000
            || !config.prg_rom_len.is_multiple_of(0x2000)
        {
            return Err(MapperError::InvalidConfiguration {
                mapper_id: 23,
                reason: "VRC2/VRC4 PRG ROM must contain at least four complete 8 KiB banks",
            });
        }
        if config.chr_rom_len == 0 && config.total_chr_ram_len() == 0 {
            return Err(MapperError::InvalidConfiguration {
                mapper_id: 23,
                reason: "VRC2/VRC4 requires CHR ROM or CHR RAM",
            });
        }
        let chr_len = config.chr_rom_len.max(config.total_chr_ram_len());
        let chr_limit = if variant == Variant::Vrc2b {
            0x40000
        } else {
            0x80000
        };
        if chr_len > chr_limit
            || !chr_len.is_multiple_of(0x0400)
            || config.chr_rom_len != 0 && config.total_chr_ram_len() != 0
        {
            return Err(MapperError::InvalidConfiguration {
                mapper_id: 23,
                reason: "VRC2/VRC4 requires one 1 KiB-granular CHR device within the variant's connected address lines",
            });
        }
        let prg_ram_len = config.total_prg_ram_len();
        let valid_prg_ram = match variant {
            Variant::Vrc2b => matches!(prg_ram_len, 0 | 0x2000),
            _ => matches!(prg_ram_len, 0 | 0x0800 | 0x2000),
        };
        if !valid_prg_ram {
            return Err(MapperError::InvalidConfiguration {
                mapper_id: 23,
                reason: "VRC2/VRC4 supports no PRG RAM, 2 KiB VRC4 RAM, or an externally decoded 8 KiB RAM",
            });
        }

        let mirroring = config.mirroring;
        Ok(Self {
            config,
            variant,
            prg_banks: [0; 2],
            prg_swap: false,
            ram_enabled: false,
            chr_low: [0; 8],
            chr_high: [0; 8],
            mirroring,
            microwire_latch: 0,
            irq_latch: 0,
            irq_counter: 0,
            irq_prescaler: 341,
            irq_enabled: false,
            irq_enable_after_ack: false,
            irq_cycle_mode: false,
            irq_pending: false,
        })
    }

    fn logical_subaddress(&self, address: u16) -> u16 {
        let (a0, a1) = match self.variant {
            // Legacy mapper 23 has to accept both VRC2b/VRC4f (A0/A1) and
            // VRC4e (A2/A3) address wiring. We check both patterns independently.
            // VRC2b/VRC4f use A0/A1: addresses match pattern 0,1,2,3
            // VRC4e uses A2/A3: addresses match pattern 0,4,8,C
            Variant::Combined => {
                let low4 = address & 0xf;
                if low4 <= 3 {
                    // VRC2b/VRC4f wiring: A0→A0, A1→A1
                    (address & 1, (address >> 1) & 1)
                } else {
                    // VRC4e wiring: A2→A0, A3→A1
                    ((address >> 2) & 1, (address >> 3) & 1)
                }
            }
            Variant::Vrc4f | Variant::Vrc2b => (address & 1, (address >> 1) & 1),
            Variant::Vrc4e => ((address >> 2) & 1, (address >> 3) & 1),
        };
        (a1 << 1) | a0
    }

    fn prg_bank_for_slot(&self, slot: usize) -> usize {
        let count = self.config.prg_rom_len / 0x2000;
        match (self.prg_swap && self.variant.has_vrc4_features(), slot) {
            (false, 0) => self.prg_banks[0] as usize & 0x1f,
            (false, 1) => self.prg_banks[1] as usize & 0x1f,
            (false, 2) => count - 2,
            (true, 0) => count - 2,
            (true, 1) => self.prg_banks[1] as usize & 0x1f,
            (true, 2) => self.prg_banks[0] as usize & 0x1f,
            (_, _) => count - 1,
        }
    }

    fn cpu_mapping(&self, address: u16) -> ReadMapping {
        if (self.ram_enabled || !self.variant.has_vrc4_features())
            && let Some(offset) = self.prg_ram_offset(address)
        {
            return ReadMapping::PrgRam(offset);
        }
        if address >= 0x8000 {
            let slot = (address as usize - 0x8000) / 0x2000;
            return ReadMapping::PrgRom(bank_offset(
                self.prg_bank_for_slot(slot),
                0x2000,
                self.config.prg_rom_len,
                address as usize & 0x1fff,
            ));
        }
        if self.variant == Variant::Vrc2b
            && self.config.total_prg_ram_len() == 0
            && (0x6000..=0x6fff).contains(&address)
        {
            return ReadMapping::Value(BusValue {
                value: self.microwire_latch,
                driven_mask: 1,
            });
        }
        ReadMapping::OpenBus
    }

    fn prg_ram_offset(&self, address: u16) -> Option<usize> {
        let len = self.config.total_prg_ram_len();
        let decoded = match len {
            0x0800 => (0x6000..0x7000).contains(&address),
            0x2000 => (0x6000..0x8000).contains(&address),
            _ => false,
        };
        decoded.then(|| (address as usize - 0x6000) % len)
    }

    fn chr_bank(&self, slot: usize) -> usize {
        usize::from(self.chr_low[slot]) | (usize::from(self.chr_high[slot]) << 4)
    }

    fn chr_mapping(&self, address: u16) -> ReadMapping {
        if address >= 0x2000 {
            return ReadMapping::OpenBus;
        }
        let slot = address as usize / 0x0400;
        let inner = address as usize & 0x03ff;
        if self.config.chr_rom_len == 0 {
            ReadMapping::ChrRam(bank_offset(
                self.chr_bank(slot),
                0x0400,
                self.config.total_chr_ram_len(),
                inner,
            ))
        } else {
            ReadMapping::ChrRom(bank_offset(
                self.chr_bank(slot),
                0x0400,
                self.config.chr_rom_len,
                inner,
            ))
        }
    }

    fn set_mirroring(&mut self, value: u8) {
        self.mirroring = match value
            & if self.variant.has_vrc4_features() {
                3
            } else {
                1
            } {
            0 => Mirroring::Vertical,
            1 => Mirroring::Horizontal,
            2 => Mirroring::OneScreenLower,
            _ => Mirroring::OneScreenUpper,
        };
    }

    fn write_register(&mut self, address: u16, value: u8) {
        let group = address & 0xf000;
        let sub = self.logical_subaddress(address);
        match group {
            0x8000 => self.prg_banks[0] = value & 0x1f,
            // Mirroring control:
            // VRC2b: at $9000-$9001 (subaddresses 0-1) per NESdev wiki
            // VRC4: at $9000 only (subaddress 0) per NESdev wiki
            0x9000 if sub < 2 => self.set_mirroring(value),
            // PRG Swap/WRAM control at $9002 only (subaddress 2) for VRC4 variants
            0x9000 if self.variant.has_vrc4_features() && sub == 2 => {
                self.ram_enabled = value & 1 != 0;
                self.prg_swap = value & 2 != 0;
            }
            0xa000 => self.prg_banks[1] = value & 0x1f,
            0xb000..=0xe000 => {
                let pair = ((group - 0xb000) / 0x1000) as usize;
                let slot = pair * 2 + usize::from(sub >= 2);
                if sub & 1 == 0 {
                    self.chr_low[slot] = value & 0x0f;
                } else {
                    self.chr_high[slot] = value
                        & if self.variant == Variant::Vrc2b {
                            0x0f
                        } else {
                            0x1f
                        };
                }
            }
            0xf000 if self.variant.has_vrc4_features() => match sub {
                0 => self.irq_latch = (self.irq_latch & 0xf0) | (value & 0x0f),
                1 => self.irq_latch = (self.irq_latch & 0x0f) | ((value & 0x0f) << 4),
                2 => {
                    self.irq_enable_after_ack = value & 1 != 0;
                    self.irq_enabled = value & 2 != 0;
                    self.irq_cycle_mode = value & 4 != 0;
                    self.irq_pending = false;
                    self.irq_prescaler = 341;
                    if self.irq_enabled {
                        self.irq_counter = self.irq_latch;
                    }
                }
                _ => {
                    self.irq_pending = false;
                    self.irq_enabled = self.irq_enable_after_ack;
                }
            },
            _ => {}
        }
    }

    fn clock_irq_counter(&mut self) {
        if self.irq_counter == 0xff {
            self.irq_counter = self.irq_latch;
            self.irq_pending = true;
        } else {
            self.irq_counter = self.irq_counter.wrapping_add(1);
        }
    }
}

impl Mapper for M023 {
    fn config(&self) -> &CartridgeConfig {
        &self.config
    }

    fn cpu_read(&mut self, address: u16) -> ReadMapping {
        self.cpu_mapping(address)
    }

    fn cpu_peek(&self, address: u16) -> ReadMapping {
        self.cpu_mapping(address)
    }

    fn cpu_write(&mut self, address: u16, value: u8, _: Option<u8>) -> WriteMapping {
        if (0x6000..=0x7fff).contains(&address) {
            if (self.ram_enabled || !self.variant.has_vrc4_features())
                && let Some(offset) = self.prg_ram_offset(address)
            {
                return WriteMapping::PrgRam(offset);
            }
            if self.variant == Variant::Vrc2b && (0x6000..=0x6fff).contains(&address) {
                self.microwire_latch = value & 1;
                return WriteMapping::Handled;
            }
            return WriteMapping::Unmapped;
        }
        if address >= 0x8000 {
            self.write_register(address, value);
            WriteMapping::Handled
        } else {
            WriteMapping::Unmapped
        }
    }

    fn ppu_read(&mut self, address: u16) -> ReadMapping {
        self.chr_mapping(address)
    }

    fn ppu_peek(&self, address: u16) -> ReadMapping {
        self.chr_mapping(address)
    }

    fn ppu_write(&mut self, address: u16, _: u8) -> WriteMapping {
        match self.chr_mapping(address) {
            ReadMapping::ChrRam(offset) => WriteMapping::ChrRam(offset),
            _ => WriteMapping::Unmapped,
        }
    }

    fn nametable_target(&self, address: u16) -> NametableTarget {
        nametable_target(self.mirroring, address)
    }

    fn clock_cpu(&mut self) {
        if !self.irq_enabled || !self.variant.has_vrc4_features() {
            return;
        }
        if self.irq_cycle_mode {
            self.clock_irq_counter();
        } else {
            self.irq_prescaler -= 3;
            if self.irq_prescaler <= 0 {
                self.irq_prescaler += 341;
                self.clock_irq_counter();
            }
        }
    }

    fn irq_pending(&self) -> bool {
        self.irq_pending
    }

    fn reset(&mut self, kind: ResetKind) {
        if kind == ResetKind::PowerOn {
            self.irq_enabled = false;
            self.irq_enable_after_ack = false;
            self.irq_pending = false;
            self.irq_prescaler = 341;
            self.prg_banks = [0; 2];
            self.prg_swap = false;
            self.ram_enabled = false;
            self.chr_low = [0; 8];
            self.chr_high = [0; 8];
            self.mirroring = self.config.mirroring;
            self.microwire_latch = 0;
            self.irq_latch = 0;
            self.irq_counter = 0;
            self.irq_cycle_mode = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TimingMode;

    fn config(submapper: u8) -> CartridgeConfig {
        CartridgeConfig {
            mapper_id: 23,
            submapper,
            prg_rom_len: 0x40000,
            chr_rom_len: if submapper == 3 { 0x40000 } else { 0x80000 },
            prg_ram_len: 0,
            prg_nvram_len: 0,
            chr_ram_len: 0,
            chr_nvram_len: 0,
            mirroring: Mirroring::Vertical,
            timing: TimingMode::Ntsc,
            has_battery: false,
            has_trainer: false,
            is_nes_2_0: true,
        }
    }

    #[test]
    fn vrc4e_uses_a2_a3_for_split_chr_registers() {
        let mut mapper = M023::new(config(2)).unwrap();
        mapper.cpu_write(0xb000, 0x0a, None);
        mapper.cpu_write(0xb004, 0x03, None);
        mapper.cpu_write(0xb008, 0x05, None);
        mapper.cpu_write(0xb00c, 0x01, None);
        assert_eq!(mapper.ppu_peek(0), ReadMapping::ChrRom(0x3a * 0x400));
        assert_eq!(mapper.ppu_peek(0x400), ReadMapping::ChrRom(0x15 * 0x400));

        mapper.cpu_write(0xb004, 0x10, None);
        assert_eq!(mapper.ppu_peek(0), ReadMapping::ChrRom(0x10a * 0x400));
    }

    #[test]
    fn vrc2_exposes_only_one_microwire_bit_and_no_vrc4_features() {
        let mut mapper = M023::new(config(3)).unwrap();
        mapper.cpu_write(0x6000, 1, None);
        assert_eq!(
            mapper.cpu_peek(0x6000),
            ReadMapping::Value(BusValue {
                value: 1,
                driven_mask: 1
            })
        );
        mapper.cpu_write(0x9002, 3, None);
        mapper.cpu_write(0xf002, 7, None);
        for _ in 0..300 {
            mapper.clock_cpu();
        }
        assert!(!mapper.irq_pending());
        assert_eq!(mapper.nametable_target(0x2800), NametableTarget::Ciram(0));
    }

    #[test]
    fn vrc4_prg_swap_and_cycle_irq_match_register_protocol() {
        let mut mapper = M023::new(config(1)).unwrap();
        mapper.cpu_write(0x8000, 3, None);
        assert_eq!(mapper.cpu_peek(0x8000), ReadMapping::PrgRom(3 * 0x2000));
        mapper.cpu_write(0x9002, 2, None);
        assert_eq!(mapper.cpu_peek(0x8000), ReadMapping::PrgRom(30 * 0x2000));
        assert_eq!(mapper.cpu_peek(0xc000), ReadMapping::PrgRom(3 * 0x2000));

        mapper.cpu_write(0xf000, 0x0e, None);
        mapper.cpu_write(0xf001, 0x0f, None);
        mapper.cpu_write(0xf002, 0x07, None);
        mapper.clock_cpu();
        mapper.clock_cpu();
        assert!(mapper.irq_pending());
        mapper.cpu_write(0xf003, 0, None);
        assert!(!mapper.irq_pending());
    }

    #[test]
    fn scanline_mode_uses_114_114_113_cpu_cycle_sequence() {
        let mut mapper = M023::new(config(1)).unwrap();
        mapper.cpu_write(0xf000, 0x0f, None);
        mapper.cpu_write(0xf001, 0x0f, None);
        mapper.cpu_write(0xf002, 0x02, None);
        for _ in 0..113 {
            mapper.clock_cpu();
        }
        assert!(!mapper.irq_pending());
        mapper.clock_cpu();
        assert!(mapper.irq_pending());
    }

    #[test]
    fn vrc4_internal_two_kib_ram_decodes_only_6000_through_6fff() {
        let mut cfg = config(1);
        cfg.prg_ram_len = 0x0800;
        let mut mapper = M023::new(cfg).unwrap();
        mapper.cpu_write(0x9002, 1, None);

        assert_eq!(mapper.cpu_peek(0x6000), ReadMapping::PrgRam(0));
        assert_eq!(mapper.cpu_peek(0x6800), ReadMapping::PrgRam(0));
        assert_eq!(mapper.cpu_peek(0x6fff), ReadMapping::PrgRam(0x07ff));
        assert_eq!(mapper.cpu_peek(0x7000), ReadMapping::OpenBus);
        assert_eq!(mapper.cpu_write(0x7000, 0xaa, None), WriteMapping::Unmapped);
    }

    #[test]
    fn vrc4_external_eight_kib_ram_decodes_the_full_window() {
        let mut cfg = config(2);
        cfg.prg_ram_len = 0x2000;
        let mut mapper = M023::new(cfg).unwrap();
        mapper.cpu_write(0x9008, 1, None);

        assert_eq!(mapper.cpu_peek(0x6000), ReadMapping::PrgRam(0));
        assert_eq!(mapper.cpu_peek(0x7fff), ReadMapping::PrgRam(0x1fff));
    }
}
