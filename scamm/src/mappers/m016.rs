use crate::{
    BusValue, CartridgeConfig, Mapper, MapperError, Mirroring, NametableTarget, ReadMapping,
    ResetKind, WriteMapping,
};

use super::{bank_offset, nametable_target};

#[derive(Debug)]
struct Eeprom24C02 {
    data: [u8; 256],
    scl: bool,
    sda: bool,
    active: bool,
    stage: u8,
    byte: u8,
    bits: u8,
    address: u8,
    ack: bool,
    ack_risen: bool,
    next_after_ack: u8,
    send_bit: i8,
    send_byte: u8,
    wait_master_ack: bool,
    master_ack: Option<bool>,
    output_low: bool,
}

impl Eeprom24C02 {
    fn new() -> Self {
        Self {
            data: [0xff; 256],
            scl: true,
            sda: true,
            active: false,
            stage: 0,
            byte: 0,
            bits: 0,
            address: 0,
            ack: false,
            ack_risen: false,
            next_after_ack: 0,
            send_bit: -1,
            send_byte: 0,
            wait_master_ack: false,
            master_ack: None,
            output_low: false,
        }
    }

    fn start(&mut self) {
        self.active = true;
        self.stage = 0;
        self.byte = 0;
        self.bits = 0;
        self.ack = false;
        self.send_bit = -1;
        self.wait_master_ack = false;
        self.master_ack = None;
        self.output_low = false;
    }

    fn set_lines(&mut self, scl: bool, sda: bool) {
        if self.scl && scl && self.sda && !sda {
            self.start();
        } else if self.scl && scl && !self.sda && sda {
            self.active = false;
            self.output_low = false;
        }

        if self.active && !self.scl && scl {
            if self.ack {
                self.ack_risen = true;
            } else if self.send_bit >= 0 {
                // The controller samples the already-held output bit.
            } else if self.wait_master_ack {
                self.master_ack = Some(!sda);
            } else {
                self.byte = (self.byte << 1) | u8::from(sda);
                self.bits += 1;
                if self.bits == 8 {
                    self.receive_byte();
                }
            }
        }

        if self.active && self.scl && !scl {
            if self.ack && self.ack_risen {
                self.ack = false;
                self.ack_risen = false;
                self.output_low = false;
                match self.next_after_ack {
                    1 => self.stage = 1,
                    2 => self.stage = 2,
                    3 => self.begin_send(),
                    _ => {}
                }
            } else if self.send_bit >= 0 {
                self.send_bit -= 1;
                if self.send_bit >= 0 {
                    self.output_low = self.send_byte & (1 << self.send_bit) == 0;
                } else {
                    self.output_low = false;
                    self.wait_master_ack = true;
                }
            } else if let Some(acknowledged) = self.master_ack.take() {
                self.wait_master_ack = false;
                if acknowledged {
                    self.address = self.address.wrapping_add(1);
                    self.begin_send();
                }
            }
        }
        self.scl = scl;
        self.sda = sda;
    }

    fn receive_byte(&mut self) {
        let value = self.byte;
        self.byte = 0;
        self.bits = 0;
        let accepted = match self.stage {
            0 if value & 0xfe == 0xa0 => {
                self.next_after_ack = if value & 1 == 0 { 1 } else { 3 };
                true
            }
            1 => {
                self.address = value;
                self.next_after_ack = 2;
                true
            }
            2 => {
                self.data[self.address as usize] = value;
                self.address = self.address.wrapping_add(1);
                self.next_after_ack = 2;
                true
            }
            _ => false,
        };
        if accepted {
            self.ack = true;
            self.ack_risen = false;
            self.output_low = true;
        }
    }

    fn begin_send(&mut self) {
        self.send_byte = self.data[self.address as usize];
        self.send_bit = 7;
        self.output_low = self.send_byte & 0x80 == 0;
    }

    fn data_out(&self) -> bool {
        !self.output_low
    }
}

/// Bandai FCG-1/FCG-2 and LZ93D50 (iNES mapper 16).
///
/// NES 2.0 submapper 4 selects the FCG register decoder and direct IRQ
/// counter. Submapper 5 selects the LZ93D50 decoder and latched IRQ counter.
/// Legacy submapper 0 responds in both register ranges and uses the LZ93D50
/// IRQ latch behavior, matching the compatible submapper-0 definition.
/// Submapper 5 supports its 24C02 through mapper-owned persistent storage.
#[derive(Debug)]
pub struct M016 {
    config: CartridgeConfig,
    chr_banks: [u8; 8],
    prg_bank: u8,
    mirroring: Mirroring,
    irq_enabled: bool,
    irq_counter: u16,
    irq_latch: u16,
    irq_pending: bool,
    eeprom: Option<Eeprom24C02>,
    eeprom_read_enabled: bool,
}

impl M016 {
    pub fn new(mut config: CartridgeConfig) -> Result<Self, MapperError> {
        if !matches!(config.submapper, 0 | 4 | 5) {
            return Err(MapperError::Unsupported {
                mapper_id: 16,
                submapper: config.submapper,
            });
        }
        if config.prg_rom_len < 0x8000
            || config.prg_rom_len > 0x40000
            || !config.prg_rom_len.is_multiple_of(0x4000)
        {
            return Err(MapperError::InvalidConfiguration {
                mapper_id: 16,
                reason: "mapper 16 PRG ROM must be 32-256 KiB in 16 KiB units",
            });
        }
        if config.chr_rom_len < 0x2000
            || config.chr_rom_len > 0x40000
            || !config.chr_rom_len.is_multiple_of(0x0400)
        {
            return Err(MapperError::InvalidConfiguration {
                mapper_id: 16,
                reason: "mapper 16 requires 8-256 KiB CHR ROM in 1 KiB units",
            });
        }
        let has_eeprom = config.submapper == 5 && config.prg_nvram_len == 0x100;
        if config.prg_nvram_len != 0 && !has_eeprom {
            return Err(MapperError::InvalidConfiguration {
                mapper_id: 16,
                reason: "mapper 16 serial EEPROM is not representable as directly mapped PRG NVRAM",
            });
        }
        // The 24C02 is serial mapper storage, not directly mapped PRG NVRAM.
        if has_eeprom {
            config.prg_nvram_len = 0;
        }
        if config.submapper == 0 && config.has_battery {
            return Err(MapperError::InvalidConfiguration {
                mapper_id: 16,
                reason: "battery-backed legacy mapper 16 may contain an unsupported serial EEPROM; use NES 2.0 board metadata",
            });
        }
        if config.prg_ram_len != 0 {
            return Err(MapperError::InvalidConfiguration {
                mapper_id: 16,
                reason: "mapper 16 does not provide directly mapped PRG RAM; Bandai WRAM boards use mapper 153",
            });
        }
        if config.total_chr_ram_len() != 0 {
            return Err(MapperError::InvalidConfiguration {
                mapper_id: 16,
                reason: "mapper 16 uses CHR ROM; CHR-RAM Bandai boards have distinct mapper IDs",
            });
        }

        let mirroring = config.mirroring;
        Ok(Self {
            config,
            chr_banks: [0; 8],
            prg_bank: 0,
            mirroring,
            irq_enabled: false,
            irq_counter: 0,
            irq_latch: 0,
            irq_pending: false,
            eeprom: has_eeprom.then(Eeprom24C02::new),
            eeprom_read_enabled: false,
        })
    }

    fn uses_latched_irq(&self) -> bool {
        self.config.submapper != 4
    }

    fn register_selected(&self, address: u16) -> bool {
        match self.config.submapper {
            4 => (0x6000..0x8000).contains(&address),
            5 => address >= 0x8000,
            // An iNES header cannot identify the ASIC. The compatibility
            // definition is therefore the union of both address decoders.
            _ => address >= 0x6000,
        }
    }

    fn cpu_mapping(&self, address: u16) -> ReadMapping {
        if (0x6000..0x8000).contains(&address)
            && self.config.submapper == 5
            && let Some(eeprom) = &self.eeprom
        {
            return ReadMapping::Value(BusValue {
                value: u8::from(self.eeprom_read_enabled && eeprom.data_out()) << 4,
                driven_mask: 0x10,
            });
        }
        if address < 0x8000 {
            return ReadMapping::OpenBus;
        }

        let bank = if address < 0xc000 {
            (self.prg_bank & 0x0f) as usize
        } else {
            self.config.prg_rom_len / 0x4000 - 1
        };
        ReadMapping::PrgRom(bank_offset(
            bank,
            0x4000,
            self.config.prg_rom_len,
            address as usize & 0x3fff,
        ))
    }

    fn chr_mapping(&self, address: u16) -> ReadMapping {
        if address >= 0x2000 {
            return ReadMapping::OpenBus;
        }
        let slot = address as usize / 0x0400;
        ReadMapping::ChrRom(bank_offset(
            self.chr_banks[slot] as usize,
            0x0400,
            self.config.chr_rom_len,
            address as usize & 0x03ff,
        ))
    }

    fn write_register(&mut self, address: u16, value: u8) {
        match address & 0x000f {
            0x0..=0x7 => self.chr_banks[(address & 7) as usize] = value,
            0x8 => self.prg_bank = value & 0x0f,
            0x9 => {
                self.mirroring = match value & 3 {
                    0 => Mirroring::Vertical,
                    1 => Mirroring::Horizontal,
                    2 => Mirroring::OneScreenLower,
                    _ => Mirroring::OneScreenUpper,
                };
            }
            0xa => {
                self.irq_pending = false;
                if self.uses_latched_irq() {
                    self.irq_counter = self.irq_latch;
                }
                self.irq_enabled = value & 1 != 0;
                // Hardware asserts immediately when enabled while holding 0.
                if self.irq_enabled && self.irq_counter == 0 {
                    self.irq_pending = true;
                }
            }
            0xb => {
                if self.uses_latched_irq() {
                    self.irq_latch = (self.irq_latch & 0xff00) | value as u16;
                } else {
                    self.irq_counter = (self.irq_counter & 0xff00) | value as u16;
                }
            }
            0xc => {
                if self.uses_latched_irq() {
                    self.irq_latch = (self.irq_latch & 0x00ff) | ((value as u16) << 8);
                } else {
                    self.irq_counter = (self.irq_counter & 0x00ff) | ((value as u16) << 8);
                }
            }
            0xd if self.config.submapper == 5 => {
                self.eeprom_read_enabled = value & 0x80 != 0;
                if let Some(eeprom) = &mut self.eeprom {
                    eeprom.set_lines(value & 0x20 != 0, value & 0x40 != 0);
                }
            }
            _ => {}
        }
    }
}

impl Mapper for M016 {
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
        if self.register_selected(address) {
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

    fn ppu_write(&mut self, _: u16, _: u8) -> WriteMapping {
        WriteMapping::Unmapped
    }

    fn nametable_target(&self, address: u16) -> NametableTarget {
        nametable_target(self.mirroring, address)
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

    fn clock_cpu(&mut self) {
        if self.irq_enabled {
            self.irq_counter = self.irq_counter.wrapping_sub(1);
            if self.irq_counter == 0xFFFF {
                self.irq_pending = true;
            }
        }
    }

    fn irq_pending(&self) -> bool {
        self.irq_pending
    }

    fn reset(&mut self, kind: ResetKind) {
        if kind == ResetKind::PowerOn {
            self.chr_banks = [0; 8];
            self.prg_bank = 0;
            self.mirroring = self.config.mirroring;
            self.irq_enabled = false;
            self.irq_pending = false;
            self.irq_counter = 0;
            self.irq_latch = 0;
            self.eeprom_read_enabled = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TimingMode;

    fn config(submapper: u8) -> CartridgeConfig {
        CartridgeConfig {
            mapper_id: 16,
            submapper,
            prg_rom_len: 0x40000,
            chr_rom_len: 0x40000,
            prg_ram_len: 0,
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

    fn i2c_lines(mapper: &mut M016, scl: bool, sda: bool) {
        let value = 0x80 | (u8::from(scl) << 5) | (u8::from(sda) << 6);
        // Every address whose low nibble is $D is an LZ93D50 mirror.
        mapper.cpu_write(0xfffd, value, None);
    }

    fn i2c_start(mapper: &mut M016) {
        i2c_lines(mapper, true, true);
        i2c_lines(mapper, true, false);
        i2c_lines(mapper, false, false);
    }

    fn i2c_stop(mapper: &mut M016) {
        i2c_lines(mapper, false, false);
        i2c_lines(mapper, true, false);
        i2c_lines(mapper, true, true);
    }

    fn i2c_write_byte(mapper: &mut M016, value: u8) {
        for bit in (0..8).rev() {
            let data = value & (1 << bit) != 0;
            i2c_lines(mapper, false, data);
            i2c_lines(mapper, true, data);
        }
        // The slave drives ACK during the ninth clock, after the falling edge
        // that follows the eighth data bit.
        i2c_lines(mapper, false, true);
        i2c_lines(mapper, true, true);
        let ReadMapping::Value(ack) = mapper.cpu_peek(0x6000) else {
            panic!("EEPROM did not expose its ACK bit");
        };
        assert_eq!(ack.resolve(0xff) & 0x10, 0);
        i2c_lines(mapper, false, true);
    }

    fn i2c_read_byte(mapper: &mut M016) -> u8 {
        let mut value = 0;
        for _ in 0..8 {
            i2c_lines(mapper, true, true);
            let ReadMapping::Value(bit) = mapper.cpu_peek(0x6000) else {
                panic!("EEPROM did not expose serial data");
            };
            value = (value << 1) | ((bit.resolve(0) >> 4) & 1);
            i2c_lines(mapper, false, true);
        }
        // NACK terminates the sequential read.
        i2c_lines(mapper, true, true);
        i2c_lines(mapper, false, true);
        value
    }

    fn i2c_write(mapper: &mut M016, address: u8, value: u8) {
        i2c_start(mapper);
        i2c_write_byte(mapper, 0xa0);
        i2c_write_byte(mapper, address);
        i2c_write_byte(mapper, value);
        i2c_stop(mapper);
    }

    fn i2c_read(mapper: &mut M016, address: u8) -> u8 {
        i2c_start(mapper);
        i2c_write_byte(mapper, 0xa0);
        i2c_write_byte(mapper, address);
        i2c_lines(mapper, true, true);
        i2c_lines(mapper, true, false); // repeated START
        i2c_lines(mapper, false, false);
        i2c_write_byte(mapper, 0xa1);
        let value = i2c_read_byte(mapper);
        i2c_stop(mapper);
        value
    }

    #[test]
    fn submapper_decoders_and_banks_are_distinct() {
        let mut fcg = M016::new(config(4)).unwrap();
        assert_eq!(fcg.cpu_write(0x8008, 3, None), WriteMapping::Unmapped);
        assert_eq!(fcg.cpu_write(0x6008, 3, None), WriteMapping::Handled);
        assert_eq!(fcg.cpu_peek(0x8000), ReadMapping::PrgRom(3 * 0x4000));

        let mut lz = M016::new(config(5)).unwrap();
        assert_eq!(lz.cpu_write(0x6008, 4, None), WriteMapping::Unmapped);
        lz.cpu_write(0xf008, 4, None);
        lz.cpu_write(0xe007, 0x21, None);
        assert_eq!(lz.cpu_peek(0x8000), ReadMapping::PrgRom(4 * 0x4000));
        assert_eq!(lz.ppu_peek(0x1c00), ReadMapping::ChrRom(0x21 * 0x400));
        assert_eq!(lz.cpu_peek(0xc000), ReadMapping::PrgRom(15 * 0x4000));
    }

    #[test]
    fn fcg_counter_is_direct_but_lz_counter_is_latched() {
        let mut fcg = M016::new(config(4)).unwrap();
        fcg.cpu_write(0x600b, 1, None);
        fcg.cpu_write(0x600a, 1, None);
        fcg.clock_cpu();
        assert!(!fcg.irq_pending());
        fcg.clock_cpu();
        assert!(fcg.irq_pending());

        let mut lz = M016::new(config(5)).unwrap();
        lz.cpu_write(0x800b, 2, None);
        lz.cpu_write(0x800a, 1, None);
        lz.cpu_write(0x800b, 9, None);
        lz.clock_cpu();
        lz.clock_cpu();
        assert!(!lz.irq_pending());
        lz.clock_cpu();
        assert!(lz.irq_pending());
        lz.cpu_write(0x800a, 0, None);
        assert!(!lz.irq_pending());
    }

    #[test]
    fn enabling_zero_counter_asserts_immediately() {
        let mut mapper = M016::new(config(5)).unwrap();
        mapper.cpu_write(0x800a, 1, None);
        assert!(mapper.irq_pending());
    }

    #[test]
    fn serial_eeprom_uses_mapper_persistence_instead_of_prg_ram() {
        let mut cfg = config(5);
        cfg.prg_nvram_len = 256;
        let mapper = M016::new(cfg).unwrap();
        assert_eq!(mapper.config.prg_nvram_len, 0);
        assert_eq!(mapper.persistent_mapper_ram().len(), 256);

        let mut legacy = config(0);
        legacy.is_nes_2_0 = false;
        legacy.has_battery = true;
        assert!(matches!(
            M016::new(legacy),
            Err(MapperError::InvalidConfiguration { mapper_id: 16, .. })
        ));
    }

    #[test]
    fn serial_eeprom_protocol_open_bus_and_save_round_trip() {
        let mut cfg = config(5);
        cfg.prg_nvram_len = 256;
        let mut mapper = M016::new(cfg.clone()).unwrap();

        let ReadMapping::Value(disabled) = mapper.cpu_peek(0x6000) else {
            panic!("serial port must partially drive the CPU bus");
        };
        assert_eq!(disabled.driven_mask, 0x10);
        assert_eq!(disabled.resolve(0xff), 0xef);

        i2c_write(&mut mapper, 0x5a, 0xc3);
        assert_eq!(i2c_read(&mut mapper, 0x5a), 0xc3);

        let saved = mapper.persistent_mapper_ram().to_vec();
        let mut restored = M016::new(cfg).unwrap();
        restored.load_persistent_mapper_ram(&saved);
        assert_eq!(i2c_read(&mut restored, 0x5a), 0xc3);
    }
}
