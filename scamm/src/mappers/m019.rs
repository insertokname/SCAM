use crate::{
    BusValue, CartridgeConfig, Mapper, MapperError, NametableTarget, ReadMapping, ResetKind,
    WriteMapping,
};

use super::{bank_offset, nametable_target};

/// Namco 129/163 (iNES mapper 19).
///
/// The implementation models the 15-bit cycle IRQ, all twelve independent
/// 1 KiB PPU banks, CIRAM substitution, 2 KiB-granular WRAM protection and
/// the time-multiplexed Namco 163 wavetable synthesizer.
#[derive(Debug)]
pub struct M019 {
    config: CartridgeConfig,
    prg_banks: [u8; 3],
    chr_banks: [u8; 8],
    nt_banks: [u8; 4],
    low_chr_rom_only: bool,
    high_chr_rom_only: bool,
    write_protect: u8,
    irq_counter: u16,
    irq_pending: bool,
    audio_ram: [u8; 0x80],
    audio_address: u8,
    audio_auto_increment: bool,
    audio_disabled: bool,
    audio_divider: u8,
    current_audio_channel: u8,
    channel_outputs: [i16; 8],
    mixed_audio: i16,
}

impl M019 {
    pub fn new(config: CartridgeConfig) -> Result<Self, MapperError> {
        if config.submapper > 5 {
            return Err(MapperError::Unsupported {
                mapper_id: 19,
                submapper: config.submapper,
            });
        }
        if config.prg_rom_len < 0x8000
            || config.prg_rom_len > 0x80000
            || !config.prg_rom_len.is_multiple_of(0x2000)
        {
            return Err(MapperError::InvalidConfiguration {
                mapper_id: 19,
                reason: "Namco 163 PRG ROM must contain at least four complete 8 KiB banks",
            });
        }
        if !matches!(config.total_prg_ram_len(), 0 | 0x2000) {
            return Err(MapperError::InvalidConfiguration {
                mapper_id: 19,
                reason: "Namco 163 supports either no PRG RAM or one 8 KiB PRG RAM device",
            });
        }
        if config.chr_rom_len == 0 && config.total_chr_ram_len() == 0 {
            return Err(MapperError::InvalidConfiguration {
                mapper_id: 19,
                reason: "Namco 163 requires CHR ROM or CHR RAM",
            });
        }
        if config.chr_rom_len > 0x40000
            || config.chr_rom_len != 0 && !config.chr_rom_len.is_multiple_of(0x0400)
            || config.total_chr_ram_len() > 0x40000
            || config.total_chr_ram_len() != 0 && !config.total_chr_ram_len().is_multiple_of(0x0400)
        {
            return Err(MapperError::InvalidConfiguration {
                mapper_id: 19,
                reason: "Namco 163 CHR ROM must use complete 1 KiB banks",
            });
        }
        if config.chr_rom_len != 0 && config.total_chr_ram_len() > 0x8000 {
            return Err(MapperError::InvalidConfiguration {
                mapper_id: 19,
                reason: "Namco 163 cannot distinguish more than 32 KiB of CHR RAM beside CHR ROM",
            });
        }

        Ok(Self {
            config,
            prg_banks: [0; 3],
            chr_banks: [0; 8],
            nt_banks: [0xe0, 0xe1, 0xe0, 0xe1],
            low_chr_rom_only: false,
            high_chr_rom_only: false,
            write_protect: 0,
            irq_counter: 0,
            irq_pending: false,
            audio_ram: [0; 0x80],
            audio_address: 0,
            audio_auto_increment: false,
            audio_disabled: false,
            audio_divider: 0,
            current_audio_channel: 7,
            channel_outputs: [0; 8],
            mixed_audio: 0,
        })
    }

    fn prg_mapping(&self, address: u16) -> ReadMapping {
        match address {
            0x6000..=0x7fff if self.config.total_prg_ram_len() != 0 => {
                ReadMapping::PrgRam((address as usize - 0x6000) % self.config.total_prg_ram_len())
            }
            0x8000..=0xffff => {
                let slot = (address as usize - 0x8000) / 0x2000;
                let bank = if slot == 3 {
                    self.config.prg_rom_len / 0x2000 - 1
                } else {
                    self.prg_banks[slot] as usize & 0x3f
                };
                ReadMapping::PrgRom(bank_offset(
                    bank,
                    0x2000,
                    self.config.prg_rom_len,
                    address as usize & 0x1fff,
                ))
            }
            _ => ReadMapping::OpenBus,
        }
    }

    fn ciram_substitution(&self, slot: usize, bank: u8) -> Option<usize> {
        let rom_only = if slot < 4 {
            self.low_chr_rom_only
        } else {
            self.high_chr_rom_only
        };
        (bank >= 0xe0
            && !rom_only
            && !(self.config.chr_rom_len != 0 && self.config.total_chr_ram_len() != 0))
            .then_some((bank as usize & 1) * 0x0400)
    }

    fn chr_mapping(&self, address: u16) -> ReadMapping {
        if address >= 0x2000 {
            return ReadMapping::OpenBus;
        }
        let slot = address as usize / 0x0400;
        let bank = self.chr_banks[slot];
        if self.ciram_substitution(slot, bank).is_some() {
            return ReadMapping::OpenBus;
        }
        let inner = address as usize & 0x03ff;
        let use_chr_ram = self.config.chr_rom_len == 0
            || (self.config.total_chr_ram_len() != 0
                && bank >= 0xe0
                && !if slot < 4 {
                    self.low_chr_rom_only
                } else {
                    self.high_chr_rom_only
                });
        if use_chr_ram {
            ReadMapping::ChrRam(bank_offset(
                bank as usize,
                0x0400,
                self.config.total_chr_ram_len(),
                inner,
            ))
        } else {
            ReadMapping::ChrRom(bank_offset(
                bank as usize,
                0x0400,
                self.config.chr_rom_len,
                inner,
            ))
        }
    }

    fn increment_audio_address(&mut self) {
        if self.audio_auto_increment {
            self.audio_address = self.audio_address.saturating_add(1).min(0x7f);
        }
    }

    fn audio_port_read(&mut self) -> u8 {
        let value = self.audio_ram[self.audio_address as usize];
        self.increment_audio_address();
        value
    }

    fn audio_port_write(&mut self, value: u8) {
        self.audio_ram[self.audio_address as usize] = value;
        self.increment_audio_address();
    }

    fn channel_count(&self) -> u8 {
        ((self.audio_ram[0x7f] >> 4) & 7) + 1
    }

    fn read_phase(&self, base: usize) -> u32 {
        self.audio_ram[base + 1] as u32
            | ((self.audio_ram[base + 3] as u32) << 8)
            | ((self.audio_ram[base + 5] as u32) << 16)
    }

    fn write_phase(&mut self, base: usize, phase: u32) {
        self.audio_ram[base + 1] = phase as u8;
        self.audio_ram[base + 3] = (phase >> 8) as u8;
        self.audio_ram[base + 5] = (phase >> 16) as u8;
    }

    fn clock_audio_channel(&mut self, channel: usize) {
        let base = 0x40 + channel * 8;
        let frequency = self.audio_ram[base] as u32
            | ((self.audio_ram[base + 2] as u32) << 8)
            | (((self.audio_ram[base + 4] & 3) as u32) << 16);
        let length = 256 - (self.audio_ram[base + 4] as u32 & 0xfc);
        let phase = (self.read_phase(base) + frequency) % (length << 16);
        self.write_phase(base, phase);

        let sample_index = ((phase >> 16) as u8).wrapping_add(self.audio_ram[base + 6]);
        let packed = self.audio_ram[(sample_index >> 1) as usize];
        let sample = if sample_index & 1 == 0 {
            packed & 0x0f
        } else {
            packed >> 4
        };
        let volume = self.audio_ram[base + 7] & 0x0f;
        self.channel_outputs[channel] = (i16::from(sample) - 8) * i16::from(volume);

        // The ASIC time-multiplexes a single DAC. Each channel's value is held
        // for its 15-cycle service slot; it does not output an arithmetic
        // average of all active channels.
        self.mixed_audio = self.channel_outputs[channel];
    }

    fn audio_gain(&self) -> f32 {
        // NES 2.0 submappers encode measured relative mixing ranges. Use each
        // range's midpoint. Submapper 2 explicitly has no expansion audio.
        match self.config.submapper {
            1 | 2 => 0.0,
            3 => 10.0_f32.powf(12.0 / 20.0),
            4 => 10.0_f32.powf(16.5 / 20.0),
            5 => 10.0_f32.powf(18.75 / 20.0),
            _ => 1.0,
        }
    }
}

impl Mapper for M019 {
    fn config(&self) -> &CartridgeConfig {
        &self.config
    }

    fn cpu_read(&mut self, address: u16) -> ReadMapping {
        match address {
            0x4800..=0x4fff => ReadMapping::Value(BusValue::full(self.audio_port_read())),
            0x5000..=0x57ff => ReadMapping::Value(BusValue::full(self.irq_counter as u8)),
            0x5800..=0x5fff => ReadMapping::Value(BusValue::full((self.irq_counter >> 8) as u8)),
            _ => self.prg_mapping(address),
        }
    }

    fn cpu_peek(&self, address: u16) -> ReadMapping {
        match address {
            0x4800..=0x4fff => {
                ReadMapping::Value(BusValue::full(self.audio_ram[self.audio_address as usize]))
            }
            0x5000..=0x57ff => ReadMapping::Value(BusValue::full(self.irq_counter as u8)),
            0x5800..=0x5fff => ReadMapping::Value(BusValue::full((self.irq_counter >> 8) as u8)),
            _ => self.prg_mapping(address),
        }
    }

    fn cpu_write(&mut self, address: u16, value: u8, _: Option<u8>) -> WriteMapping {
        match address {
            0x4800..=0x4fff => self.audio_port_write(value),
            0x5000..=0x57ff => {
                self.irq_counter = (self.irq_counter & 0xff00) | u16::from(value);
                self.irq_pending = false;
            }
            0x5800..=0x5fff => {
                self.irq_counter = (self.irq_counter & 0x00ff) | (u16::from(value) << 8);
                self.irq_pending = false;
            }
            0x6000..=0x7fff if self.config.total_prg_ram_len() != 0 => {
                let window = (address as usize - 0x6000) / 0x0800;
                let globally_enabled = self.write_protect & 0xf0 == 0x40;
                if globally_enabled && self.write_protect & (1 << window) == 0 {
                    return WriteMapping::PrgRam(
                        (address as usize - 0x6000) % self.config.total_prg_ram_len(),
                    );
                }
                return WriteMapping::Handled;
            }
            0x8000..=0xbfff => {
                let slot = (address as usize - 0x8000) / 0x0800;
                self.chr_banks[slot] = value;
            }
            0xc000..=0xdfff => {
                let slot = (address as usize - 0xc000) / 0x0800;
                self.nt_banks[slot] = value;
            }
            0xe000..=0xe7ff => {
                self.prg_banks[0] = value & 0x3f;
                self.audio_disabled = value & 0x40 != 0;
            }
            0xe800..=0xefff => {
                self.prg_banks[1] = value & 0x3f;
                self.low_chr_rom_only = value & 0x40 != 0;
                self.high_chr_rom_only = value & 0x80 != 0;
            }
            0xf000..=0xf7ff => self.prg_banks[2] = value & 0x3f,
            0xf800..=0xffff => {
                self.write_protect = value;
                self.audio_address = value & 0x7f;
                self.audio_auto_increment = value & 0x80 != 0;
            }
            _ => return WriteMapping::Unmapped,
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
        if self.ppu_ciram_target(address).is_some() {
            return WriteMapping::Handled;
        }
        match self.chr_mapping(address) {
            ReadMapping::ChrRam(offset) => WriteMapping::ChrRam(offset),
            _ => WriteMapping::Unmapped,
        }
    }

    fn ppu_ciram_target(&self, address: u16) -> Option<usize> {
        if address >= 0x2000 {
            return None;
        }
        let slot = address as usize / 0x0400;
        self.ciram_substitution(slot, self.chr_banks[slot])
            .map(|base| base | (address as usize & 0x03ff))
    }

    fn nametable_target(&self, address: u16) -> NametableTarget {
        if !(0x2000..0x3f00).contains(&address) {
            return nametable_target(self.config.mirroring, address);
        }
        let offset = address.wrapping_sub(0x2000) as usize & 0x0fff;
        let bank = self.nt_banks[offset / 0x0400];
        let inner = offset & 0x03ff;
        if bank >= 0xe0 {
            NametableTarget::Ciram(((bank as usize & 1) * 0x0400) | inner)
        } else if self.config.chr_rom_len != 0 {
            NametableTarget::ChrRom(bank_offset(
                bank as usize,
                0x0400,
                self.config.chr_rom_len,
                inner,
            ))
        } else {
            NametableTarget::ChrRam(bank_offset(
                bank as usize,
                0x0400,
                self.config.total_chr_ram_len(),
                inner,
            ))
        }
    }

    fn persistent_mapper_ram(&self) -> &[u8] {
        if self.config.has_battery {
            &self.audio_ram
        } else {
            &[]
        }
    }

    fn load_persistent_mapper_ram(&mut self, bytes: &[u8]) {
        if self.config.has_battery {
            let len = bytes.len().min(self.audio_ram.len());
            self.audio_ram[..len].copy_from_slice(&bytes[..len]);
        }
    }

    fn clock_cpu(&mut self) {
        if self.irq_counter & 0x8000 != 0 && self.irq_counter & 0x7fff != 0x7fff {
            self.irq_counter = self.irq_counter.wrapping_add(1);
            if self.irq_counter & 0x7fff == 0x7fff {
                self.irq_pending = true;
            }
        }

        if !self.audio_disabled && self.config.submapper != 2 {
            self.audio_divider += 1;
            if self.audio_divider == 15 {
                self.audio_divider = 0;
                self.clock_audio_channel(self.current_audio_channel as usize);
                let first = 8 - self.channel_count();
                self.current_audio_channel = if self.current_audio_channel <= first {
                    7
                } else {
                    self.current_audio_channel - 1
                };
            }
        }
    }

    fn irq_pending(&self) -> bool {
        self.irq_pending
    }

    fn expansion_audio_sample(&self) -> f32 {
        if self.audio_disabled {
            0.0
        } else {
            self.mixed_audio as f32 / 2048.0 * self.audio_gain()
        }
    }

    fn reset(&mut self, kind: ResetKind) {
        if kind == ResetKind::PowerOn {
            self.irq_counter = 0;
            self.irq_pending = false;
            self.audio_divider = 0;
            self.current_audio_channel = 7;
            self.channel_outputs = [0; 8];
            self.mixed_audio = 0;
            self.prg_banks = [0; 3];
            self.chr_banks = [0; 8];
            self.nt_banks = [0xe0, 0xe1, 0xe0, 0xe1];
            self.low_chr_rom_only = false;
            self.high_chr_rom_only = false;
            self.write_protect = 0;
            self.audio_address = 0;
            self.audio_auto_increment = false;
            self.audio_disabled = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Mirroring, TimingMode};

    fn config() -> CartridgeConfig {
        CartridgeConfig {
            mapper_id: 19,
            submapper: 5,
            prg_rom_len: 0x80000,
            chr_rom_len: 0x40000,
            prg_ram_len: 0x2000,
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
    fn maps_all_prg_and_nametable_banks() {
        let mut mapper = M019::new(config()).unwrap();
        mapper.cpu_write(0x8000, 5, None);
        mapper.cpu_write(0xe000, 3, None);
        mapper.cpu_write(0xe800, 4, None);
        mapper.cpu_write(0xf000, 5, None);
        assert_eq!(mapper.ppu_peek(0), ReadMapping::ChrRom(5 * 0x400));
        assert_eq!(mapper.cpu_peek(0x8000), ReadMapping::PrgRom(3 * 0x2000));
        assert_eq!(mapper.cpu_peek(0xa000), ReadMapping::PrgRom(4 * 0x2000));
        assert_eq!(mapper.cpu_peek(0xc000), ReadMapping::PrgRom(5 * 0x2000));

        mapper.cpu_write(0xc000, 7, None);
        assert_eq!(
            mapper.nametable_target(0x2123),
            NametableTarget::ChrRom(7 * 0x400 + 0x123)
        );
        mapper.cpu_write(0xc000, 0xe1, None);
        assert_eq!(
            mapper.nametable_target(0x2123),
            NametableTarget::Ciram(0x523)
        );
    }

    #[test]
    fn substitutes_ciram_until_disabled_by_e800() {
        let mut mapper = M019::new(config()).unwrap();
        mapper.cpu_write(0x8000, 0xe1, None);
        assert_eq!(mapper.ppu_ciram_target(0x0123), Some(0x0523));
        mapper.cpu_write(0xe800, 0x40, None);
        assert_eq!(mapper.ppu_ciram_target(0x0123), None);
        assert_eq!(mapper.ppu_peek(0x0123), ReadMapping::ChrRom(0x38523));
    }

    #[test]
    fn irq_saturates_and_is_acknowledged_by_counter_writes() {
        let mut mapper = M019::new(config()).unwrap();
        mapper.cpu_write(0x5000, 0xfe, None);
        mapper.cpu_write(0x5800, 0xff, None);
        mapper.clock_cpu();
        assert!(mapper.irq_pending());
        assert_eq!(
            mapper.cpu_peek(0x5000),
            ReadMapping::Value(BusValue::full(0xff))
        );
        mapper.clock_cpu();
        assert_eq!(
            mapper.cpu_peek(0x5000),
            ReadMapping::Value(BusValue::full(0xff))
        );
        mapper.cpu_write(0x5000, 0, None);
        assert!(!mapper.irq_pending());
    }

    #[test]
    fn write_protection_and_audio_port_autoincrement_are_exact() {
        let mut mapper = M019::new(config()).unwrap();
        assert_eq!(mapper.cpu_write(0x6000, 1, None), WriteMapping::Handled);
        mapper.cpu_write(0xf800, 0x40, None);
        assert_eq!(mapper.cpu_write(0x6000, 1, None), WriteMapping::PrgRam(0));
        mapper.cpu_write(0xf800, 0x41, None);
        assert_eq!(mapper.cpu_write(0x6000, 1, None), WriteMapping::Handled);

        mapper.cpu_write(0xf800, 0x80, None);
        mapper.cpu_write(0x4800, 0x12, None);
        mapper.cpu_write(0x4800, 0x34, None);
        mapper.cpu_write(0xf800, 0, None);
        assert_eq!(
            mapper.cpu_read(0x4800),
            ReadMapping::Value(BusValue::full(0x12))
        );
        mapper.cpu_write(0xf800, 1, None);
        assert_eq!(
            mapper.cpu_read(0x4800),
            ReadMapping::Value(BusValue::full(0x34))
        );

        mapper.cpu_write(0xf800, 0xff, None);
        mapper.cpu_write(0x4800, 0x56, None);
        mapper.cpu_write(0x4800, 0x78, None);
        assert_eq!(mapper.audio_address, 0x7f);
        assert_eq!(mapper.audio_ram[0x7f], 0x78);
    }

    #[test]
    fn serial_dac_holds_each_channel_instead_of_averaging_channels() {
        let mut mapper = M019::new(config()).unwrap();
        // Two channels. Channel 7 reads waveform nibble 15; channel 6 reads 0.
        mapper.audio_ram[0] = 0x0f;
        mapper.audio_ram[1] = 0x00;
        mapper.audio_ram[0x7e] = 0;
        mapper.audio_ram[0x7f] = 0x1f;
        mapper.audio_ram[0x76] = 2;
        mapper.audio_ram[0x77] = 0x0f;

        for _ in 0..15 {
            mapper.clock_cpu();
        }
        let channel_7 = mapper.expansion_audio_sample();
        assert!(channel_7 > 0.0);
        for _ in 0..14 {
            mapper.clock_cpu();
            assert_eq!(mapper.expansion_audio_sample(), channel_7);
        }
        mapper.clock_cpu();
        assert!(mapper.expansion_audio_sample() < 0.0);
    }

    #[test]
    fn battery_backs_the_full_internal_chip_ram() {
        let mut cfg = config();
        cfg.has_battery = true;
        let mut mapper = M019::new(cfg.clone()).unwrap();
        mapper.cpu_write(0xf800, 0x7f, None);
        mapper.cpu_write(0x4800, 0xa5, None);
        let saved = mapper.persistent_mapper_ram().to_vec();
        assert_eq!(saved.len(), 0x80);

        let mut restored = M019::new(cfg).unwrap();
        restored.load_persistent_mapper_ram(&saved);
        restored.cpu_write(0xf800, 0x7f, None);
        assert_eq!(
            restored.cpu_peek(0x4800),
            ReadMapping::Value(BusValue::full(0xa5))
        );
    }
}
