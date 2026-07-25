use crate::{
    CartridgeConfig, Mapper, MapperError, Mirroring, NametableTarget, ReadMapping, ResetKind,
    WriteMapping,
};

use super::{bank_count, bank_offset, nametable_target};

#[derive(Debug)]
struct Mmc3Core {
    config: CartridgeConfig,
    bank_select: u8,
    banks: [u8; 8],
    mirroring: Mirroring,
    ram_protect: u8,
    irq_latch: u8,
    irq_counter: u8,
    irq_reload: bool,
    irq_enabled: bool,
    irq_pending: bool,
    last_a12: bool,
    a12_low_since: Option<u64>,
    mc_acc_divider: u8,
    t9552_pattern: u8,
}

impl Mmc3Core {
    fn new(config: CartridgeConfig) -> Result<Self, MapperError> {
        if config.prg_rom_len < 0x8000
            || config.prg_rom_len > 0x80000
            || !config.prg_rom_len.is_multiple_of(0x2000)
        {
            return Err(MapperError::InvalidConfiguration {
                mapper_id: config.mapper_id,
                reason: "MMC3-family PRG ROM must contain 32-512 KiB in complete 8 KiB banks",
            });
        }
        let chr_len = config.chr_rom_len.max(config.total_chr_ram_len());
        if chr_len == 0
            || chr_len > 0x40000
            || !chr_len.is_multiple_of(0x0400)
            || config.chr_rom_len != 0 && config.total_chr_ram_len() != 0
        {
            return Err(MapperError::InvalidConfiguration {
                mapper_id: config.mapper_id,
                reason: "MMC3-family CHR storage must be one 1 KiB-granular ROM or RAM device up to 256 KiB",
            });
        }
        let mirroring = config.mirroring;
        Ok(Self {
            config,
            bank_select: 0,
            banks: [0; 8],
            mirroring,
            // iNES compatibility starts RAM enabled. Software can immediately
            // apply the actual hardware protection state through $A001.
            ram_protect: 0x80,
            irq_latch: 0,
            irq_counter: 0,
            irq_reload: false,
            irq_enabled: false,
            irq_pending: false,
            last_a12: false,
            a12_low_since: None,
            mc_acc_divider: 0,
            t9552_pattern: 0,
        })
    }

    fn scramble_prg_bank(&self, bank: usize) -> usize {
        if self.config.submapper != 5 {
            return bank;
        }
        // Each row is the signal routed to the corresponding file-order
        // address line A14..A17. Pattern 2 is the mapper-4.5 ROM file order.
        const SIGNALS: [[u8; 4]; 4] = [
            [16, 17, 15, 14],
            [17, 16, 14, 15],
            [14, 15, 16, 17],
            [15, 14, 17, 16],
        ];
        let mut output = bank & !0x1e;
        for (row, signal) in SIGNALS[(self.t9552_pattern & 3) as usize]
            .iter()
            .copied()
            .enumerate()
        {
            output |= ((bank >> (signal - 13)) & 1) << (row + 1);
        }
        output
    }

    fn scramble_chr_bank(&self, bank: usize) -> usize {
        if self.config.submapper != 5 {
            return bank;
        }
        const SIGNALS: [[u8; 6]; 8] = [
            [15, 12, 16, 17, 14, 13],
            [14, 15, 13, 12, 17, 16],
            [12, 13, 14, 15, 16, 17],
            [16, 14, 12, 13, 17, 15],
            [15, 13, 17, 16, 12, 14],
            [14, 12, 15, 16, 17, 13],
            [13, 16, 14, 15, 12, 17],
            [12, 15, 16, 17, 13, 14],
        ];
        let mut output = bank & !0xfc;
        for (row, signal) in SIGNALS[(self.t9552_pattern & 7) as usize]
            .iter()
            .copied()
            .enumerate()
        {
            output |= ((bank >> (signal - 10)) & 1) << (row + 2);
        }
        output
    }

    fn prg_bank_for_slot(&self, slot: usize, dxrom: bool) -> usize {
        let count = bank_count(self.config.prg_rom_len, 0x2000);
        if dxrom {
            return match slot {
                0 => (self.banks[6] & 0x0f) as usize,
                1 => (self.banks[7] & 0x0f) as usize,
                2 => count - 2,
                _ => count - 1,
            };
        }
        let r6 = (self.banks[6] & 0x3f) as usize;
        let r7 = (self.banks[7] & 0x3f) as usize;
        match (self.bank_select & 0x40 != 0, slot) {
            (false, 0) => r6,
            (false, 1) => r7,
            (false, 2) => count - 2,
            (true, 0) => count - 2,
            (true, 1) => r7,
            (true, 2) => r6,
            (_, _) => count - 1,
        }
    }

    fn cpu_mapping(&self, address: u16, dxrom: bool) -> ReadMapping {
        if dxrom && self.config.submapper == 1 && address >= 0x8000 {
            return ReadMapping::PrgRom((address as usize - 0x8000) % self.config.prg_rom_len);
        }
        if !dxrom && let Some(mapping) = self.ram_read_mapping(address) {
            return mapping;
        }
        if address >= 0x8000 {
            let slot = (address as usize - 0x8000) / 0x2000;
            ReadMapping::PrgRom(bank_offset(
                self.scramble_prg_bank(self.prg_bank_for_slot(slot, dxrom)),
                0x2000,
                self.config.prg_rom_len,
                address as usize & 0x1fff,
            ))
        } else {
            ReadMapping::OpenBus
        }
    }

    fn ram_read_mapping(&self, address: u16) -> Option<ReadMapping> {
        if self.config.total_prg_ram_len() == 0 {
            return None;
        }
        if self.config.submapper == 1 {
            if !(0x7000..0x8000).contains(&address) || self.bank_select & 0x20 == 0 {
                return None;
            }
            let half = usize::from(address & 0x0200 != 0);
            let readable = self.ram_protect & (0x20 << (half * 2)) != 0;
            return readable.then(|| {
                ReadMapping::PrgRam((address as usize & 0x03ff) % self.config.total_prg_ram_len())
            });
        }
        ((0x6000..0x8000).contains(&address) && self.ram_protect & 0x80 != 0).then(|| {
            ReadMapping::PrgRam((address as usize - 0x6000) % self.config.total_prg_ram_len())
        })
    }

    fn ram_write_mapping(&self, address: u16) -> Option<WriteMapping> {
        self.ram_read_mapping(address).and_then(|mapping| {
            let ReadMapping::PrgRam(offset) = mapping else {
                return None;
            };
            let writable = if self.config.submapper == 1 {
                let half = usize::from(address & 0x0200 != 0);
                self.ram_protect & (0x10 << (half * 2)) != 0
            } else {
                self.ram_protect & 0x40 == 0
            };
            writable.then_some(WriteMapping::PrgRam(offset))
        })
    }

    fn chr_bank_for_slot(&self, slot: usize, dxrom: bool) -> usize {
        if dxrom {
            return match slot {
                0 => (self.banks[0] & 0x3e) as usize,
                1 => (self.banks[0] & 0x3e) as usize + 1,
                2 => (self.banks[1] & 0x3e) as usize,
                3 => (self.banks[1] & 0x3e) as usize + 1,
                4..=7 => (self.banks[slot - 2] & 0x3f) as usize,
                _ => 0,
            };
        }
        let normal_slot = if self.bank_select & 0x80 == 0 {
            slot
        } else {
            (slot + 4) & 7
        };
        match normal_slot {
            0 => (self.banks[0] & 0xfe) as usize,
            1 => (self.banks[0] & 0xfe) as usize + 1,
            2 => (self.banks[1] & 0xfe) as usize,
            3 => (self.banks[1] & 0xfe) as usize + 1,
            4..=7 => self.banks[normal_slot - 2] as usize,
            _ => 0,
        }
    }

    fn chr_mapping(&self, address: u16, dxrom: bool) -> ReadMapping {
        if address >= 0x2000 {
            return ReadMapping::OpenBus;
        }
        let len = if self.config.chr_rom_len == 0 {
            self.config.total_chr_ram_len()
        } else {
            self.config.chr_rom_len
        };
        let slot = address as usize / 0x0400;
        let offset = bank_offset(
            self.scramble_chr_bank(self.chr_bank_for_slot(slot, dxrom)),
            0x0400,
            len,
            address as usize & 0x03ff,
        );
        if self.config.chr_rom_len == 0 {
            ReadMapping::ChrRam(offset)
        } else {
            ReadMapping::ChrRom(offset)
        }
    }

    fn write_register(&mut self, address: u16, value: u8) {
        match (address & 0xe001, self.config.submapper) {
            (0x8000, 1) => {
                self.bank_select = value;
                if value & 0x20 == 0 {
                    self.ram_protect = 0;
                }
            }
            (0x8000, _) => self.bank_select = value,
            (0x8001, _) => self.banks[(self.bank_select & 7) as usize] = value,
            (0xa000, _)
                if self.config.submapper != 2 && self.config.mirroring != Mirroring::FourScreen =>
            {
                self.mirroring = if value & 1 == 0 {
                    Mirroring::Horizontal
                } else {
                    Mirroring::Vertical
                };
            }
            (0xa001, 1) if self.bank_select & 0x20 != 0 => self.ram_protect = value,
            (0xa001, _) => self.ram_protect = value,
            (0xc000, _) => self.irq_latch = value,
            (0xc001, _) => {
                self.irq_counter = 0;
                self.irq_reload = true;
            }
            (0xe000, _) => {
                self.irq_enabled = false;
                self.irq_pending = false;
            }
            (0xe001, _) => self.irq_enabled = true,
            _ => {}
        }
    }

    fn clock_irq(&mut self) {
        let previous = self.irq_counter;
        let was_reload = self.irq_reload;
        if was_reload || self.irq_counter == 0 {
            self.irq_counter = self.irq_latch;
        } else {
            self.irq_counter = self.irq_counter.wrapping_sub(1);
        }
        self.irq_reload = false;

        let fire = if self.config.submapper == 4 {
            self.irq_counter == 0 && (was_reload || previous != 0)
        } else {
            self.irq_counter == 0
        };
        if self.irq_enabled && fire {
            self.irq_pending = true;
        }
    }

    fn notify_ppu_address(&mut self, address: u16, ppu_cycle: u64) {
        let a12 = address & 0x1000 != 0;
        if self.config.submapper == 3 {
            if self.last_a12 && !a12 {
                self.mc_acc_divider = (self.mc_acc_divider + 1) & 7;
                if self.mc_acc_divider == 0 {
                    self.clock_irq();
                }
            }
            self.last_a12 = a12;
            return;
        }
        if !a12 {
            if self.last_a12 || self.a12_low_since.is_none() {
                self.a12_low_since = Some(ppu_cycle);
            }
        } else if !self.last_a12
            && self
                .a12_low_since
                .is_some_and(|since| ppu_cycle.wrapping_sub(since) >= 8)
        {
            self.clock_irq();
            self.a12_low_since = None;
        }
        self.last_a12 = a12;
    }

    fn reset(&mut self, kind: ResetKind) {
        if kind == ResetKind::PowerOn {
            self.irq_enabled = false;
            self.irq_pending = false;
            self.irq_reload = false;
            self.irq_counter = 0;
            self.last_a12 = false;
            self.a12_low_since = None;
            self.mc_acc_divider = 0;
            self.t9552_pattern = 0;
            self.bank_select = 0;
            self.banks = [0; 8];
            self.ram_protect = 0x80;
            self.mirroring = self.config.mirroring;
        }
    }
}

/// Nintendo MMC3/MMC6 (mapper 4).
#[derive(Debug)]
pub struct M004(Mmc3Core);

impl M004 {
    pub fn new(config: CartridgeConfig) -> Result<Self, MapperError> {
        if config.submapper > 5 {
            return Err(MapperError::Unsupported {
                mapper_id: 4,
                submapper: config.submapper,
            });
        }
        if config.submapper == 1 && config.total_prg_ram_len() != 0x0400 {
            return Err(MapperError::InvalidConfiguration {
                mapper_id: 4,
                reason: "MMC6 submapper 1 requires exactly 1 KiB of internal PRG RAM",
            });
        }
        if config.submapper != 1 && !matches!(config.total_prg_ram_len(), 0 | 0x2000) {
            return Err(MapperError::InvalidConfiguration {
                mapper_id: 4,
                reason: "MMC3 boards support either no PRG RAM or one 8 KiB PRG RAM device",
            });
        }
        Ok(Self(Mmc3Core::new(config)?))
    }
}

impl Mapper for M004 {
    fn config(&self) -> &CartridgeConfig {
        &self.0.config
    }
    fn cpu_read(&mut self, address: u16) -> ReadMapping {
        self.0.cpu_mapping(address, false)
    }
    fn cpu_peek(&self, address: u16) -> ReadMapping {
        self.0.cpu_mapping(address, false)
    }
    fn cpu_write(&mut self, address: u16, value: u8, _: Option<u8>) -> WriteMapping {
        if self.0.config.submapper == 5 && (0x5000..0x6000).contains(&address) {
            self.0.t9552_pattern = value & 7;
            return WriteMapping::Handled;
        }
        if let Some(mapping) = self.0.ram_write_mapping(address) {
            return mapping;
        }
        if address >= 0x8000 {
            self.0.write_register(address, value);
            WriteMapping::Handled
        } else {
            WriteMapping::Unmapped
        }
    }
    fn ppu_read(&mut self, address: u16) -> ReadMapping {
        self.0.chr_mapping(address, false)
    }
    fn ppu_peek(&self, address: u16) -> ReadMapping {
        self.0.chr_mapping(address, false)
    }
    fn ppu_write(&mut self, address: u16, _: u8) -> WriteMapping {
        match self.0.chr_mapping(address, false) {
            ReadMapping::ChrRam(offset) => WriteMapping::ChrRam(offset),
            _ => WriteMapping::Unmapped,
        }
    }
    fn nametable_target(&self, address: u16) -> NametableTarget {
        nametable_target(self.0.mirroring, address)
    }
    fn notify_ppu_address(&mut self, address: u16, cycle: u64, _: bool) {
        self.0.notify_ppu_address(address, cycle);
    }
    fn irq_pending(&self) -> bool {
        self.0.irq_pending
    }
    fn reset(&mut self, kind: ResetKind) {
        self.0.reset(kind);
    }
}

/// Namco 108/118 and Tengen MIMIC-1 (mapper 206).
#[derive(Debug)]
pub struct M206(Mmc3Core);

impl M206 {
    pub fn new(config: CartridgeConfig) -> Result<Self, MapperError> {
        if config.submapper > 1 {
            return Err(MapperError::Unsupported {
                mapper_id: 206,
                submapper: config.submapper,
            });
        }
        if config.prg_rom_len > 0x20000
            || config.chr_rom_len == 0
            || config.chr_rom_len > 0x10000
            || !config.chr_rom_len.is_multiple_of(0x0400)
            || config.total_prg_ram_len() != 0
            || config.total_chr_ram_len() != 0
        {
            return Err(MapperError::InvalidConfiguration {
                mapper_id: 206,
                reason: "Namco 108 supports at most 128 KiB PRG ROM and 64 KiB CHR ROM, with no RAM",
            });
        }
        if config.submapper == 1 && config.prg_rom_len != 0x8000 {
            return Err(MapperError::InvalidConfiguration {
                mapper_id: 206,
                reason: "mapper 206 submapper 1 is the hardwired unbanked 32 KiB PRG board",
            });
        }
        Ok(Self(Mmc3Core::new(config)?))
    }
}

impl Mapper for M206 {
    fn config(&self) -> &CartridgeConfig {
        &self.0.config
    }
    fn cpu_read(&mut self, address: u16) -> ReadMapping {
        self.0.cpu_mapping(address, true)
    }
    fn cpu_peek(&self, address: u16) -> ReadMapping {
        self.0.cpu_mapping(address, true)
    }
    fn cpu_write(&mut self, address: u16, value: u8, _: Option<u8>) -> WriteMapping {
        match address & 0xe001 {
            0x8000 => {
                self.0.bank_select = value & 7;
                WriteMapping::Handled
            }
            0x8001 => {
                self.0.banks[(self.0.bank_select & 7) as usize] = value & 0x3f;
                WriteMapping::Handled
            }
            _ => WriteMapping::Unmapped,
        }
    }
    fn ppu_read(&mut self, address: u16) -> ReadMapping {
        self.0.chr_mapping(address, true)
    }
    fn ppu_peek(&self, address: u16) -> ReadMapping {
        self.0.chr_mapping(address, true)
    }
    fn ppu_write(&mut self, _: u16, _: u8) -> WriteMapping {
        WriteMapping::Unmapped
    }
    fn nametable_target(&self, address: u16) -> NametableTarget {
        nametable_target(self.0.config.mirroring, address)
    }
    fn reset(&mut self, kind: ResetKind) {
        self.0.reset(kind);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TimingMode;

    fn config(mapper_id: u16, submapper: u8) -> CartridgeConfig {
        CartridgeConfig {
            mapper_id,
            submapper,
            prg_rom_len: 0x40000,
            chr_rom_len: 0x20000,
            prg_ram_len: if mapper_id == 206 {
                0
            } else if submapper == 1 {
                0x0400
            } else {
                0x2000
            },
            prg_nvram_len: 0,
            chr_ram_len: 0,
            chr_nvram_len: 0,
            mirroring: Mirroring::Horizontal,
            timing: TimingMode::Ntsc,
            has_battery: false,
            has_trainer: false,
            is_nes_2_0: true,
        }
    }

    #[test]
    fn mmc3_maps_prg_modes_and_chr_inversion() {
        let mut mapper = M004::new(config(4, 0)).unwrap();
        mapper.cpu_write(0x8000, 6, None);
        mapper.cpu_write(0x8001, 3, None);
        assert_eq!(mapper.cpu_peek(0x8000), ReadMapping::PrgRom(0x6000));
        mapper.cpu_write(0x8000, 0x46, None);
        assert_eq!(mapper.cpu_peek(0xc000), ReadMapping::PrgRom(0x6000));

        mapper.cpu_write(0x8000, 0, None);
        mapper.cpu_write(0x8001, 5, None);
        assert_eq!(mapper.ppu_peek(0), ReadMapping::ChrRom(0x1000));
        mapper.cpu_write(0x8000, 0x80, None);
        assert_eq!(mapper.ppu_peek(0x1000), ReadMapping::ChrRom(0x1000));
    }

    #[test]
    fn mmc3_filters_a12_and_holds_level_irq_until_acknowledged() {
        let mut mapper = M004::new(config(4, 0)).unwrap();
        mapper.cpu_write(0xc000, 1, None);
        mapper.cpu_write(0xe001, 0, None);
        mapper.notify_ppu_address(0x0000, 0, true);
        mapper.notify_ppu_address(0x1000, 8, true);
        assert!(!mapper.irq_pending());
        mapper.notify_ppu_address(0x0000, 9, true);
        mapper.notify_ppu_address(0x1000, 17, true);
        assert!(mapper.irq_pending());
        mapper.cpu_write(0xe000, 0, None);
        assert!(!mapper.irq_pending());
    }

    #[test]
    fn mapper206_has_fixed_last_two_banks() {
        let mut cfg = config(206, 0);
        cfg.prg_rom_len = 0x20000;
        cfg.chr_rom_len = 0x10000;
        let mut mapper = M206::new(cfg).unwrap();
        mapper.cpu_write(0x8000, 6, None);
        mapper.cpu_write(0x8001, 2, None);
        assert_eq!(mapper.cpu_peek(0x8000), ReadMapping::PrgRom(0x4000));
        assert_eq!(mapper.cpu_peek(0xc000), ReadMapping::PrgRom(0x1c000));
    }

    #[test]
    fn mapper206_submapper_one_keeps_32k_prg_unbanked() {
        let mut cfg = config(206, 1);
        cfg.prg_rom_len = 0x8000;
        cfg.chr_rom_len = 0x10000;
        let mut mapper = M206::new(cfg).unwrap();
        mapper.cpu_write(0x8000, 6, None);
        mapper.cpu_write(0x8001, 3, None);
        assert_eq!(mapper.cpu_peek(0x8000), ReadMapping::PrgRom(0));
        assert_eq!(mapper.cpu_peek(0xa000), ReadMapping::PrgRom(0x2000));
    }

    #[test]
    fn mmc3_submapper_two_has_hardwired_mirroring() {
        let mut mapper = M004::new(config(4, 2)).unwrap();
        mapper.cpu_write(0xa000, 0, None);
        assert_eq!(
            mapper.nametable_target(0x2800),
            NametableTarget::Ciram(0x400)
        );
    }

    #[test]
    fn mc_acc_divides_falling_a12_and_nec_zero_reload_fires_once() {
        let mut mc_acc = M004::new(config(4, 3)).unwrap();
        mc_acc.cpu_write(0xc000, 0, None);
        mc_acc.cpu_write(0xe001, 0, None);
        for edge in 0..8 {
            mc_acc.notify_ppu_address(0x1000, edge * 2, true);
            mc_acc.notify_ppu_address(0x0000, edge * 2 + 1, true);
        }
        assert!(mc_acc.irq_pending());

        let mut nec = M004::new(config(4, 4)).unwrap();
        nec.cpu_write(0xc000, 0, None);
        nec.cpu_write(0xe001, 0, None);
        nec.cpu_write(0xc001, 0, None);
        nec.notify_ppu_address(0, 0, true);
        nec.notify_ppu_address(0x1000, 8, true);
        assert!(nec.irq_pending());
        nec.cpu_write(0xe000, 0, None);
        nec.cpu_write(0xe001, 0, None);
        nec.notify_ppu_address(0, 9, true);
        nec.notify_ppu_address(0x1000, 17, true);
        assert!(!nec.irq_pending());
    }

    #[test]
    fn mmc6_disabled_halves_are_open_bus_and_each_half_has_separate_protection() {
        let mut mapper = M004::new(config(4, 1)).unwrap();
        assert_eq!(mapper.cpu_peek(0x7000), ReadMapping::OpenBus);
        mapper.cpu_write(0x8000, 0x20, None);
        mapper.cpu_write(0xa001, 0x20, None); // lower half read-only
        assert_eq!(mapper.cpu_peek(0x7000), ReadMapping::PrgRam(0));
        assert_eq!(mapper.cpu_peek(0x7200), ReadMapping::OpenBus);
        assert_eq!(mapper.cpu_write(0x7000, 0, None), WriteMapping::Unmapped);

        mapper.cpu_write(0xa001, 0xf0, None);
        assert_eq!(mapper.cpu_peek(0x73ff), ReadMapping::PrgRam(0x3ff));
        assert_eq!(mapper.cpu_write(0x7000, 0, None), WriteMapping::PrgRam(0));
        assert_eq!(
            mapper.cpu_write(0x7200, 0, None),
            WriteMapping::PrgRam(0x200)
        );
    }

    #[test]
    fn t9552_power_scrambling_and_pattern_two_file_order_are_exact() {
        let mut mapper = M004::new(config(4, 5)).unwrap();
        mapper.cpu_write(0x8000, 6, None);
        mapper.cpu_write(0x8001, 5, None);
        assert_eq!(mapper.cpu_peek(0x8000), ReadMapping::PrgRom(9 * 0x2000));

        assert_eq!(mapper.cpu_write(0x5000, 2, None), WriteMapping::Handled);
        assert_eq!(mapper.cpu_peek(0x8000), ReadMapping::PrgRom(5 * 0x2000));
        mapper.cpu_write(0x8000, 2, None);
        mapper.cpu_write(0x8001, 0x15, None);
        assert_eq!(mapper.ppu_peek(0x1000), ReadMapping::ChrRom(0x15 * 0x400));
    }

    #[test]
    fn mapper206_rejects_ram_and_non_32k_fixed_prg_variant() {
        let mut ram = config(206, 0);
        ram.prg_rom_len = 0x20000;
        ram.chr_rom_len = 0x10000;
        ram.prg_ram_len = 0x2000;
        assert!(matches!(
            M206::new(ram),
            Err(MapperError::InvalidConfiguration { mapper_id: 206, .. })
        ));

        let mut fixed = config(206, 1);
        fixed.prg_rom_len = 0x10000;
        fixed.chr_rom_len = 0x10000;
        assert!(matches!(
            M206::new(fixed),
            Err(MapperError::InvalidConfiguration { mapper_id: 206, .. })
        ));
    }
}
