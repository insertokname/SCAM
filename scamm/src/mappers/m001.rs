use crate::{
    BusValue, CartridgeConfig, Mapper, MapperError, Mirroring, NametableTarget, ReadMapping,
    ResetKind, WriteMapping,
};

use super::{bank_count, bank_offset, nametable_target};

#[derive(Debug)]
struct Eeprom93C46 {
    data: [u8; 128],
    cs: bool,
    clock: bool,
    data_out: bool,
    input: u32,
    input_bits: u8,
    write_address: Option<u8>,
    write_value: u16,
    write_bits: u8,
    read_value: u16,
    read_bit: i8,
    read_started: bool,
    write_enabled: bool,
}

impl Eeprom93C46 {
    fn new() -> Self {
        Self {
            data: [0xff; 128],
            cs: false,
            clock: false,
            data_out: true,
            input: 0,
            input_bits: 0,
            write_address: None,
            write_value: 0,
            write_bits: 0,
            read_value: 0,
            read_bit: -1,
            read_started: false,
            write_enabled: false,
        }
    }

    fn reset_transaction(&mut self) {
        self.data_out = true;
        self.input = 0;
        self.input_bits = 0;
        self.write_address = None;
        self.write_value = 0;
        self.write_bits = 0;
        self.read_bit = -1;
        self.read_started = false;
    }

    fn set_lines(&mut self, cs: bool, clock: bool, data_in: bool) {
        if !cs || !self.cs {
            self.reset_transaction();
        }

        if cs && !self.clock && clock {
            if self.write_address.is_some() {
                self.write_value = (self.write_value << 1) | u16::from(data_in);
                self.write_bits += 1;
                if self.write_bits == 16 {
                    if self.write_enabled {
                        let address = self.write_address.unwrap() as usize * 2;
                        self.data[address] = (self.write_value >> 8) as u8;
                        self.data[address + 1] = self.write_value as u8;
                    }
                    self.write_address = None;
                    self.data_out = true;
                }
            } else if self.read_bit < 0 {
                self.input = (self.input << 1) | u32::from(data_in);
                self.input_bits += 1;
                if self.input_bits == 9 {
                    self.decode_command();
                }
            }
        }

        // The next serial output bit changes on the falling edge and remains
        // held for the following rising-edge sample.
        if cs && self.clock && !clock && self.read_bit >= 0 {
            if self.read_started {
                self.read_started = false;
            } else {
                self.read_bit -= 1;
                self.data_out = if self.read_bit >= 0 {
                    self.read_value & (1 << self.read_bit) != 0
                } else {
                    true
                };
            }
        }
        self.cs = cs;
        self.clock = clock;
    }

    fn decode_command(&mut self) {
        let command = self.input as u16;
        self.input = 0;
        self.input_bits = 0;
        if command & 0x100 == 0 {
            return;
        }
        let opcode = (command >> 6) & 3;
        let address = (command & 0x3f) as u8;
        match opcode {
            0 => match address >> 4 {
                0 => self.write_enabled = false,
                3 => self.write_enabled = true,
                _ => {}
            },
            1 => {
                self.write_address = Some(address);
                self.write_value = 0;
                self.write_bits = 0;
            }
            2 => {
                let offset = address as usize * 2;
                self.read_value = u16::from_be_bytes([self.data[offset], self.data[offset + 1]]);
                self.read_bit = 15;
                self.read_started = true;
                self.data_out = self.read_value & 0x8000 != 0;
            }
            3 if self.write_enabled => {
                let offset = address as usize * 2;
                self.data[offset] = 0xff;
                self.data[offset + 1] = 0xff;
            }
            _ => {}
        }
    }
}

/// Nintendo MMC1/SxROM (iNES mapper 1).
#[derive(Debug)]
pub struct M001 {
    config: CartridgeConfig,
    shift: u8,
    control: u8,
    chr_bank_0: u8,
    chr_bank_1: u8,
    prg_bank: u8,
    cpu_cycle: u64,
    last_serial_write_cycle: Option<u64>,
    eeprom: Option<Eeprom93C46>,
}

impl M001 {
    pub fn new(config: CartridgeConfig) -> Result<Self, MapperError> {
        if !matches!(config.submapper, 0 | 5 | 6 | 7) {
            return Err(MapperError::Unsupported {
                mapper_id: 1,
                submapper: config.submapper,
            });
        }
        if config.prg_rom_len < 0x8000 || !config.prg_rom_len.is_multiple_of(0x4000) {
            return Err(MapperError::InvalidConfiguration {
                mapper_id: 1,
                reason: "MMC1 PRG ROM must contain at least two 16 KiB banks",
            });
        }
        if config.prg_rom_len > 0x80000 {
            return Err(MapperError::InvalidConfiguration {
                mapper_id: 1,
                reason: "MMC1 SxROM supports at most 512 KiB PRG ROM",
            });
        }
        let chr_len = config.chr_rom_len.max(config.total_chr_ram_len());
        if config.chr_rom_len != 0 && config.total_chr_ram_len() != 0
            || config.submapper != 6
                && (chr_len == 0 || chr_len > 0x20000 || !chr_len.is_multiple_of(0x1000))
        {
            return Err(MapperError::InvalidConfiguration {
                mapper_id: 1,
                reason: "SxROM requires one 4 KiB-granular CHR ROM/RAM device up to 128 KiB",
            });
        }
        if config.submapper == 5
            && (config.prg_rom_len != 0x8000 || config.total_prg_ram_len() != 0)
        {
            return Err(MapperError::InvalidConfiguration {
                mapper_id: 1,
                reason: "MMC1 submapper 5 requires fixed 32 KiB PRG ROM and no PRG RAM",
            });
        }
        if config.submapper != 6
            && !matches!(config.total_prg_ram_len(), 0 | 0x2000 | 0x4000 | 0x8000)
        {
            return Err(MapperError::InvalidConfiguration {
                mapper_id: 1,
                reason: "SxROM PRG RAM must be 0, 8, 16, or 32 KiB",
            });
        }
        if config.submapper == 6 && config.total_prg_ram_len() > 0x8000 {
            return Err(MapperError::InvalidConfiguration {
                mapper_id: 1,
                reason: "2ME supports at most 32 KiB of PRG RAM in addition to its 93LC46 EEPROM",
            });
        }
        if config.submapper == 7
            && !matches!(
                config.mirroring,
                Mirroring::Horizontal | Mirroring::Vertical
            )
        {
            return Err(MapperError::InvalidConfiguration {
                mapper_id: 1,
                reason: "MMC1 submapper 7 has hardwired horizontal or vertical mirroring",
            });
        }
        let has_eeprom = config.submapper == 6;
        Ok(Self {
            config,
            shift: 0x10,
            control: 0x0c,
            chr_bank_0: 0,
            chr_bank_1: 0,
            prg_bank: 0,
            cpu_cycle: 0,
            last_serial_write_cycle: None,
            eeprom: has_eeprom.then(Eeprom93C46::new),
        })
    }

    fn mirroring(&self) -> Mirroring {
        if self.config.submapper == 7 {
            return self.config.mirroring;
        }
        match self.control & 3 {
            0 => Mirroring::OneScreenLower,
            1 => Mirroring::OneScreenUpper,
            2 => Mirroring::Vertical,
            _ => Mirroring::Horizontal,
        }
    }

    fn prg_ram_enabled(&self) -> bool {
        if self.prg_bank & 0x10 != 0 || self.config.total_prg_ram_len() == 0 {
            return false;
        }
        if self.config.submapper == 6 {
            return self.chr_bank_0 & 0x10 == 0;
        }
        let chr_len = self.config.chr_rom_len.max(self.config.total_chr_ram_len());
        let snrom = self.config.prg_rom_len <= 0x40000
            && chr_len == 0x2000
            && self.config.total_prg_ram_len() == 0x2000;
        !snrom || self.chr_bank_0 & 0x10 == 0
    }

    fn outer_prg_bank(&self) -> usize {
        if self.config.prg_rom_len > 0x40000 {
            ((self.chr_bank_0 as usize >> 4) & 1) * 0x40000
        } else {
            0
        }
    }

    fn cpu_mapping(&self, address: u16) -> ReadMapping {
        if self.config.submapper == 6
            && (0x6000..0x8000).contains(&address)
            && self.prg_bank & 0x10 == 0
            && self.chr_bank_0 & 0x10 != 0
        {
            return ReadMapping::Value(BusValue {
                value: u8::from(self.eeprom.as_ref().is_some_and(|eeprom| eeprom.data_out)),
                driven_mask: 0x01,
            });
        }
        if (0x6000..0x8000).contains(&address) && self.prg_ram_enabled() {
            let total = self.config.total_prg_ram_len();
            let ram_bank = match total {
                0x4000
                    if self.config.chr_rom_len.max(self.config.total_chr_ram_len()) >= 0x4000 =>
                {
                    (self.chr_bank_0 as usize >> 4) & 1
                }
                0x4000 => (self.chr_bank_0 as usize >> 3) & 1,
                0x8000 => (self.chr_bank_0 as usize >> 2) & 3,
                _ => 0,
            };
            return ReadMapping::PrgRam((ram_bank * 0x2000 + address as usize - 0x6000) % total);
        }
        if address < 0x8000 {
            return ReadMapping::OpenBus;
        }

        if self.config.submapper == 5 {
            return ReadMapping::PrgRom(address as usize - 0x8000);
        }

        let mode = (self.control >> 2) & 3;
        let selected = (self.prg_bank & 0x0f) as usize;
        let total_banks = bank_count(self.config.prg_rom_len.min(0x40000), 0x4000);
        let inner = address as usize & 0x3fff;
        let bank = match mode {
            0 | 1 => ((selected & !1) + usize::from(address >= 0xc000)) % total_banks,
            2 => {
                if address < 0xc000 {
                    0
                } else {
                    selected
                }
            }
            _ => {
                if address < 0xc000 {
                    selected
                } else {
                    total_banks - 1
                }
            }
        };
        ReadMapping::PrgRom(
            (self.outer_prg_bank()
                + bank_offset(bank, 0x4000, self.config.prg_rom_len.min(0x40000), inner))
                % self.config.prg_rom_len,
        )
    }

    fn chr_mapping(&self, address: u16) -> ReadMapping {
        if address >= 0x2000 {
            return ReadMapping::OpenBus;
        }
        let chr_len = if self.config.chr_rom_len == 0 {
            self.config.total_chr_ram_len()
        } else {
            self.config.chr_rom_len
        };
        let bank_4k = if self.control & 0x10 == 0 {
            ((self.chr_bank_0 as usize & !1) + usize::from(address >= 0x1000))
                % bank_count(chr_len, 0x1000)
        } else if address < 0x1000 {
            self.chr_bank_0 as usize
        } else {
            self.chr_bank_1 as usize
        };
        let offset = bank_offset(bank_4k, 0x1000, chr_len, address as usize & 0x0fff);
        if self.config.chr_rom_len == 0 {
            ReadMapping::ChrRam(offset)
        } else {
            ReadMapping::ChrRom(offset)
        }
    }

    fn commit_serial(&mut self, address: u16, value: u8) {
        match address {
            0x8000..=0x9fff => self.control = value,
            0xa000..=0xbfff => self.chr_bank_0 = value,
            0xc000..=0xdfff => self.chr_bank_1 = value,
            0xe000..=0xffff => self.prg_bank = value,
            _ => {}
        }
        self.update_eeprom_lines();
    }

    fn update_eeprom_lines(&mut self) {
        if let Some(eeprom) = &mut self.eeprom {
            let cs = self.control & 3 == 1;
            let clock = self.chr_bank_0 & 2 != 0;
            let data_in = self.control & 0x10 != 0 && self.chr_bank_0 & 1 != 0;
            eeprom.set_lines(cs, clock, data_in);
        }
    }
}

impl Mapper for M001 {
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
        if (0x6000..0x8000).contains(&address)
            && self.prg_ram_enabled()
            && let ReadMapping::PrgRam(offset) = self.cpu_mapping(address)
        {
            return WriteMapping::PrgRam(offset);
        }
        if address < 0x8000 {
            return WriteMapping::Unmapped;
        }

        // MMC1 ignores non-reset serial writes on the CPU cycle immediately
        // following another accepted serial write. This filters RMW writes.
        if value & 0x80 != 0 {
            self.shift = 0x10;
            self.control |= 0x0c;
            self.last_serial_write_cycle = Some(self.cpu_cycle);
            return WriteMapping::Handled;
        }
        if self
            .last_serial_write_cycle
            .is_some_and(|cycle| cycle == self.cpu_cycle || cycle.wrapping_add(1) == self.cpu_cycle)
        {
            return WriteMapping::Handled;
        }

        self.last_serial_write_cycle = Some(self.cpu_cycle);
        let complete = self.shift & 1 != 0;
        self.shift = (self.shift >> 1) | ((value & 1) << 4);
        if complete {
            let register = self.shift & 0x1f;
            self.commit_serial(address, register);
            self.shift = 0x10;
        }
        WriteMapping::Handled
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

    fn persistent_mapper_ram(&self) -> &[u8] {
        self.eeprom.as_ref().map_or(&[], |eeprom| &eeprom.data)
    }

    fn load_persistent_mapper_ram(&mut self, bytes: &[u8]) {
        if let Some(eeprom) = &mut self.eeprom {
            let len = bytes.len().min(eeprom.data.len());
            eeprom.data[..len].copy_from_slice(&bytes[..len]);
        }
    }

    fn nametable_target(&self, address: u16) -> NametableTarget {
        nametable_target(self.mirroring(), address)
    }

    fn clock_cpu(&mut self) {
        self.cpu_cycle = self.cpu_cycle.wrapping_add(1);
    }

    fn reset(&mut self, kind: ResetKind) {
        if kind == ResetKind::PowerOn {
            self.shift = 0x10;
            self.control = 0x0c;
            self.chr_bank_0 = 0;
            self.chr_bank_1 = 0;
            self.prg_bank = 0;
            self.cpu_cycle = 0;
            self.last_serial_write_cycle = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TimingMode;

    fn config() -> CartridgeConfig {
        CartridgeConfig {
            mapper_id: 1,
            submapper: 0,
            prg_rom_len: 0x40000,
            chr_rom_len: 0x20000,
            prg_ram_len: 0x2000,
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

    fn serial_write(mapper: &mut M001, address: u16, value: u8) {
        for bit in 0..5 {
            mapper.cpu_write(address, value >> bit, None);
            mapper.clock_cpu();
            mapper.clock_cpu();
        }
    }

    fn eeprom_lines(mapper: &mut M001, cs: bool, clock: bool, data: bool) {
        let control = u8::from(cs) | (u8::from(data) << 4);
        let chr = 0x10 | u8::from(data) | (u8::from(clock) << 1);
        mapper.commit_serial(0x8000, control);
        mapper.commit_serial(0xa000, chr);
    }

    fn eeprom_bit(mapper: &mut M001, bit: bool) {
        eeprom_lines(mapper, true, false, bit);
        eeprom_lines(mapper, true, true, bit);
        eeprom_lines(mapper, true, false, bit);
    }

    fn eeprom_command(mapper: &mut M001, command: u16) {
        eeprom_lines(mapper, false, false, false);
        eeprom_lines(mapper, true, false, false);
        for bit in (0..9).rev() {
            eeprom_bit(mapper, command & (1 << bit) != 0);
        }
    }

    fn eeprom_write_word(mapper: &mut M001, address: u8, value: u16) {
        eeprom_command(mapper, 0x130); // EWEN
        eeprom_lines(mapper, false, false, false);
        eeprom_command(mapper, 0x140 | u16::from(address));
        for bit in (0..16).rev() {
            eeprom_bit(mapper, value & (1 << bit) != 0);
        }
        eeprom_lines(mapper, false, false, false);
    }

    fn eeprom_read_word(mapper: &mut M001, address: u8) -> u16 {
        eeprom_command(mapper, 0x180 | u16::from(address));
        let mut value = 0;
        for _ in 0..16 {
            let ReadMapping::Value(bus) = mapper.cpu_peek(0x6000) else {
                panic!("EEPROM did not drive its serial output");
            };
            value = (value << 1) | u16::from(bus.resolve(0) & 1);
            eeprom_lines(mapper, true, true, false);
            eeprom_lines(mapper, true, false, false);
        }
        eeprom_lines(mapper, false, false, false);
        value
    }

    #[test]
    fn serial_register_selects_prg_bank() {
        let mut mapper = M001::new(config()).unwrap();
        serial_write(&mut mapper, 0xe000, 5);
        assert_eq!(mapper.cpu_peek(0x8000), ReadMapping::PrgRom(5 * 0x4000));
        assert_eq!(mapper.cpu_peek(0xc000), ReadMapping::PrgRom(15 * 0x4000));
    }

    #[test]
    fn reset_forces_fixed_last_bank_mode() {
        let mut mapper = M001::new(config()).unwrap();
        serial_write(&mut mapper, 0x8000, 0);
        mapper.cpu_write(0x8000, 0x80, None);
        assert_eq!(mapper.control & 0x0c, 0x0c);
    }

    #[test]
    fn ignores_immediately_consecutive_serial_write() {
        let mut mapper = M001::new(config()).unwrap();
        mapper.cpu_write(0xe000, 1, None);
        mapper.cpu_write(0xe000, 0, None);
        assert_eq!(mapper.shift, 0x18);
        mapper.clock_cpu();
        mapper.cpu_write(0xe000, 1, None);
        assert_eq!(mapper.shift, 0x18);
    }

    #[test]
    fn fixed_and_hardwired_submappers_ignore_unconnected_controls() {
        let mut fixed = config();
        fixed.submapper = 5;
        fixed.prg_rom_len = 0x8000;
        fixed.prg_ram_len = 0;
        let mut fixed = M001::new(fixed).unwrap();
        serial_write(&mut fixed, 0xe000, 0x0f);
        assert_eq!(fixed.cpu_peek(0x8000), ReadMapping::PrgRom(0));
        assert_eq!(fixed.cpu_peek(0xffff), ReadMapping::PrgRom(0x7fff));

        let mut hardwired = config();
        hardwired.submapper = 7;
        hardwired.mirroring = Mirroring::Vertical;
        let mut hardwired = M001::new(hardwired).unwrap();
        serial_write(&mut hardwired, 0x8000, 3);
        assert_eq!(
            hardwired.nametable_target(0x2800),
            NametableTarget::Ciram(0)
        );
    }

    #[test]
    fn microwire_eeprom_protocol_and_persistence_round_trip() {
        let mut cfg = config();
        cfg.submapper = 6;
        let mut mapper = M001::new(cfg.clone()).unwrap();

        eeprom_write_word(&mut mapper, 0x12, 0xa55a);
        assert_eq!(eeprom_read_word(&mut mapper, 0x12), 0xa55a);
        assert_eq!(mapper.persistent_mapper_ram().len(), 128);

        let saved = mapper.persistent_mapper_ram().to_vec();
        let mut restored = M001::new(cfg).unwrap();
        restored.load_persistent_mapper_ram(&saved);
        assert_eq!(eeprom_read_word(&mut restored, 0x12), 0xa55a);
    }

    #[test]
    fn reset_button_preserves_mmc1_latches() {
        let mut mapper = M001::new(config()).unwrap();
        serial_write(&mut mapper, 0xe000, 5);
        mapper.reset(ResetKind::ResetButton);
        assert_eq!(mapper.cpu_peek(0x8000), ReadMapping::PrgRom(5 * 0x4000));
    }
}
