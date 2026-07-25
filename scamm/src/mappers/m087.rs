use crate::{
    CartridgeConfig, Mapper, MapperError, Mirroring, NametableTarget, ReadMapping, ResetKind,
    WriteMapping,
};

use super::{map_chr_read, nametable_target};

/// 74x139 + 74x74/J87 boards (iNES mapper 87).
#[derive(Debug)]
pub struct M087 {
    config: CartridgeConfig,
    chr_bank: usize,
}

impl M087 {
    pub fn new(config: CartridgeConfig) -> Result<Self, MapperError> {
        if config.submapper != 0 {
            return Err(MapperError::Unsupported {
                mapper_id: 87,
                submapper: config.submapper,
            });
        }
        if !matches!(config.prg_rom_len, 0x4000 | 0x8000) {
            return Err(MapperError::InvalidConfiguration {
                mapper_id: 87,
                reason: "J87 requires 16 KiB or 32 KiB PRG ROM",
            });
        }
        if !matches!(config.chr_rom_len, 0x4000 | 0x8000) || config.total_chr_ram_len() != 0 {
            return Err(MapperError::InvalidConfiguration {
                mapper_id: 87,
                reason: "J87 requires 16 KiB or 32 KiB CHR ROM",
            });
        }
        if config.total_prg_ram_len() != 0 {
            return Err(MapperError::InvalidConfiguration {
                mapper_id: 87,
                reason: "J87 has no PRG RAM",
            });
        }
        if !matches!(
            config.mirroring,
            Mirroring::Horizontal | Mirroring::Vertical
        ) {
            return Err(MapperError::InvalidConfiguration {
                mapper_id: 87,
                reason: "J87 mirroring must be hardwired horizontal or vertical",
            });
        }

        Ok(Self {
            config,
            chr_bank: 0,
        })
    }

    fn cpu_mapping(&self, address: u16) -> ReadMapping {
        if address >= 0x8000 {
            ReadMapping::PrgRom((address as usize - 0x8000) % self.config.prg_rom_len)
        } else {
            ReadMapping::OpenBus
        }
    }
}

impl Mapper for M087 {
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
            // J87's two CHR address bits are reversed relative to mapper 101.
            self.chr_bank = (((value & 1) << 1) | ((value & 2) >> 1)) as usize;
            WriteMapping::Handled
        } else {
            WriteMapping::Unmapped
        }
    }

    fn ppu_read(&mut self, address: u16) -> ReadMapping {
        map_chr_read(&self.config, address, self.chr_bank)
    }

    fn ppu_peek(&self, address: u16) -> ReadMapping {
        map_chr_read(&self.config, address, self.chr_bank)
    }

    fn ppu_write(&mut self, _: u16, _: u8) -> WriteMapping {
        WriteMapping::Unmapped
    }

    fn nametable_target(&self, address: u16) -> NametableTarget {
        nametable_target(self.config.mirroring, address)
    }

    fn reset(&mut self, kind: ResetKind) {
        if kind == ResetKind::PowerOn {
            self.chr_bank = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Mirroring, TimingMode};

    fn config(prg_rom_len: usize) -> CartridgeConfig {
        CartridgeConfig {
            mapper_id: 87,
            submapper: 0,
            prg_rom_len,
            chr_rom_len: 0x8000,
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
    fn swaps_chr_select_bits() {
        let mut mapper = M087::new(config(0x8000)).unwrap();
        mapper.cpu_write(0x6000, 1, None);
        assert_eq!(mapper.ppu_peek(0), ReadMapping::ChrRom(2 * 0x2000));
        mapper.cpu_write(0x7fff, 2, None);
        assert_eq!(mapper.ppu_peek(0), ReadMapping::ChrRom(0x2000));
    }

    #[test]
    fn mirrors_16k_prg_through_both_cpu_halves() {
        let mapper = M087::new(config(0x4000)).unwrap();
        assert_eq!(mapper.cpu_peek(0x8000), ReadMapping::PrgRom(0));
        assert_eq!(mapper.cpu_peek(0xc000), ReadMapping::PrgRom(0));
        assert_eq!(mapper.cpu_peek(0xffff), ReadMapping::PrgRom(0x3fff));
    }

    #[test]
    fn has_no_bus_conflict_at_ram_range_register() {
        let mut mapper = M087::new(config(0x8000)).unwrap();
        mapper.cpu_write(0x6000, 3, Some(0));
        assert_eq!(mapper.ppu_peek(0), ReadMapping::ChrRom(3 * 0x2000));
    }

    #[test]
    fn boundaries_fixed_mirroring_resets_and_negative_features_are_exact() {
        let mut mapper = M087::new(config(0x8000)).unwrap();
        assert_eq!(mapper.cpu_peek(0x7fff), ReadMapping::OpenBus);
        assert_eq!(mapper.cpu_peek(0x8000), ReadMapping::PrgRom(0));
        assert_eq!(mapper.cpu_peek(0xffff), ReadMapping::PrgRom(0x7fff));
        assert_eq!(mapper.cpu_write(0x5fff, 3, None), WriteMapping::Unmapped);
        assert_eq!(mapper.cpu_write(0x6000, 3, None), WriteMapping::Handled);
        assert_eq!(mapper.cpu_write(0x8000, 0, None), WriteMapping::Unmapped);
        assert_eq!(mapper.ppu_peek(0x1fff), ReadMapping::ChrRom(0x7fff));
        assert_eq!(mapper.ppu_peek(0x2000), ReadMapping::OpenBus);
        assert_eq!(mapper.ppu_write(0, 0), WriteMapping::Unmapped);
        assert_eq!(mapper.nametable_target(0x2800), NametableTarget::Ciram(0));
        assert!(!mapper.irq_pending());
        assert_eq!(mapper.expansion_audio_sample(), 0.0);

        mapper.reset(ResetKind::ResetButton);
        assert_eq!(mapper.ppu_peek(0), ReadMapping::ChrRom(3 * 0x2000));
        mapper.reset(ResetKind::PowerOn);
        assert_eq!(mapper.ppu_peek(0), ReadMapping::ChrRom(0));

        let mut ram = config(0x8000);
        ram.prg_ram_len = 0x2000;
        assert!(matches!(
            M087::new(ram),
            Err(MapperError::InvalidConfiguration { mapper_id: 87, .. })
        ));
        let mut variant = config(0x8000);
        variant.submapper = 1;
        assert!(matches!(
            M087::new(variant),
            Err(MapperError::Unsupported {
                mapper_id: 87,
                submapper: 1
            })
        ));
    }
}
