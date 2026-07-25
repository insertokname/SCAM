use crate::{
    CartridgeConfig, Mapper, MapperError, Mirroring, NametableTarget, ReadMapping, ResetKind,
    WriteMapping,
};

use super::{bank_offset, nametable_target};

/// Sunsoft FME-7 / 5A / 5B compatible superset (iNES mapper 69).
#[derive(Debug)]
pub struct M069 {
    config: CartridgeConfig,
    command: u8,
    chr_banks: [u8; 8],
    prg_banks: [u8; 3],
    window_bank: u8,
    mirroring: Mirroring,
    irq_counter_enabled: bool,
    irq_enabled: bool,
    irq_counter: u16,
    irq_pending: bool,
    audio_select: u8,
    audio_registers: [u8; 16],
    audio_divider: u8,
    tone_counter: [u16; 3],
    tone_output: [bool; 3],
    noise_counter: u8,
    noise_lfsr: u32,
    noise_output: bool,
    envelope_counter: u16,
    envelope_level: u8,
    envelope_direction: i8,
    envelope_holding: bool,
}

impl M069 {
    pub fn new(config: CartridgeConfig) -> Result<Self, MapperError> {
        if config.submapper != 0 {
            return Err(MapperError::Unsupported {
                mapper_id: 69,
                submapper: config.submapper,
            });
        }
        if config.prg_rom_len < 0x8000
            || config.prg_rom_len > 0x80000
            || !config.prg_rom_len.is_multiple_of(0x2000)
        {
            return Err(MapperError::InvalidConfiguration {
                mapper_id: 69,
                reason: "FME-7 PRG ROM must be 32-512 KiB in 8 KiB units",
            });
        }
        if config.chr_rom_len < 0x2000
            || config.chr_rom_len > 0x40000
            || !config.chr_rom_len.is_multiple_of(0x0400)
            || config.total_chr_ram_len() != 0
        {
            return Err(MapperError::InvalidConfiguration {
                mapper_id: 69,
                reason: "FME-7 requires 8-256 KiB CHR ROM in 1 KiB units",
            });
        }
        if config.total_prg_ram_len() > 0x80000
            || config.total_prg_ram_len() != 0 && !config.total_prg_ram_len().is_multiple_of(0x2000)
        {
            return Err(MapperError::InvalidConfiguration {
                mapper_id: 69,
                reason: "FME-7 exposes at most 512 KiB of banked PRG RAM in complete 8 KiB banks",
            });
        }

        let mirroring = config.mirroring;
        Ok(Self {
            config,
            command: 0,
            chr_banks: [0; 8],
            prg_banks: [0; 3],
            window_bank: 0,
            mirroring,
            irq_counter_enabled: false,
            irq_enabled: false,
            irq_counter: 0,
            irq_pending: false,
            audio_select: 0,
            audio_registers: [0; 16],
            audio_divider: 0,
            tone_counter: [0; 3],
            tone_output: [false; 3],
            noise_counter: 0,
            noise_lfsr: 0x1ffff,
            noise_output: false,
            envelope_counter: 0,
            envelope_level: 31,
            envelope_direction: -1,
            envelope_holding: false,
        })
    }

    fn window_mapping(&self, address: u16) -> ReadMapping {
        debug_assert!((0x6000..0x8000).contains(&address));
        let bank = (self.window_bank & 0x3f) as usize;
        let inner = address as usize - 0x6000;
        if self.window_bank & 0x40 == 0 {
            ReadMapping::PrgRom(bank_offset(bank, 0x2000, self.config.prg_rom_len, inner))
        } else if self.window_bank & 0x80 != 0 && self.config.total_prg_ram_len() != 0 {
            ReadMapping::PrgRam(bank_offset(
                bank,
                0x2000,
                self.config.total_prg_ram_len(),
                inner,
            ))
        } else {
            ReadMapping::OpenBus
        }
    }

    fn cpu_mapping(&self, address: u16) -> ReadMapping {
        if (0x6000..0x8000).contains(&address) {
            return self.window_mapping(address);
        }
        if address < 0x8000 {
            return ReadMapping::OpenBus;
        }

        let slot = (address as usize - 0x8000) / 0x2000;
        let bank = if slot < 3 {
            self.prg_banks[slot] as usize
        } else {
            self.config.prg_rom_len / 0x2000 - 1
        };
        ReadMapping::PrgRom(bank_offset(
            bank,
            0x2000,
            self.config.prg_rom_len,
            address as usize & 0x1fff,
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

    fn write_parameter(&mut self, value: u8) {
        match self.command {
            0x0..=0x7 => self.chr_banks[self.command as usize] = value,
            0x8 => self.window_bank = value,
            0x9..=0xb => self.prg_banks[(self.command - 9) as usize] = value & 0x3f,
            0xc => {
                self.mirroring = match value & 3 {
                    0 => Mirroring::Vertical,
                    1 => Mirroring::Horizontal,
                    2 => Mirroring::OneScreenLower,
                    _ => Mirroring::OneScreenUpper,
                };
            }
            0xd => {
                self.irq_enabled = value & 1 != 0;
                self.irq_counter_enabled = value & 0x80 != 0;
                self.irq_pending = false;
            }
            0xe => self.irq_counter = (self.irq_counter & 0xff00) | value as u16,
            0xf => self.irq_counter = (self.irq_counter & 0x00ff) | ((value as u16) << 8),
            _ => unreachable!(),
        }
    }

    fn write_audio_register(&mut self, value: u8) {
        if self.audio_select & 0xf0 != 0 {
            return;
        }
        let register = (self.audio_select & 0x0f) as usize;
        self.audio_registers[register] = match register {
            1 | 3 | 5 => value & 0x0f,
            6 => value & 0x1f,
            8..=10 => value & 0x1f,
            13 => value & 0x0f,
            _ => value,
        };
        if register == 13 {
            self.envelope_counter = 0;
            self.envelope_holding = false;
            if value & 0x04 != 0 {
                self.envelope_level = 0;
                self.envelope_direction = 1;
            } else {
                self.envelope_level = 31;
                self.envelope_direction = -1;
            }
        }
    }

    fn clock_audio_generators(&mut self) {
        for channel in 0..3 {
            let period = (u16::from(self.audio_registers[channel * 2 + 1]) << 8)
                | u16::from(self.audio_registers[channel * 2]);
            let period = period.max(1);
            self.tone_counter[channel] = self.tone_counter[channel].saturating_add(1);
            if self.tone_counter[channel] >= period {
                self.tone_counter[channel] = 0;
                self.tone_output[channel] = !self.tone_output[channel];
            }
        }

        let noise_period = self.audio_registers[6].max(1);
        self.noise_counter = self.noise_counter.saturating_add(1);
        if self.noise_counter >= noise_period {
            self.noise_counter = 0;
            let feedback = ((self.noise_lfsr >> 16) ^ (self.noise_lfsr >> 13)) & 1;
            self.noise_lfsr = ((self.noise_lfsr << 1) | feedback) & 0x1ffff;
            self.noise_output = self.noise_lfsr & 1 != 0;
        }

        if !self.envelope_holding {
            let period =
                (u16::from(self.audio_registers[12]) << 8) | u16::from(self.audio_registers[11]);
            self.envelope_counter = self.envelope_counter.saturating_add(1);
            if self.envelope_counter >= period.max(1) {
                self.envelope_counter = 0;
                self.step_envelope();
            }
        }
    }

    fn step_envelope(&mut self) {
        let next = self.envelope_level as i8 + self.envelope_direction;
        if (0..=31).contains(&next) {
            self.envelope_level = next as u8;
            return;
        }
        let shape = self.audio_registers[13];
        if shape & 0x08 == 0 {
            self.envelope_level = 0;
            self.envelope_holding = true;
        } else if shape & 0x01 != 0 {
            if shape & 0x02 != 0 {
                self.envelope_level = 31 - self.envelope_level;
            }
            self.envelope_holding = true;
        } else if shape & 0x02 != 0 {
            self.envelope_direction = -self.envelope_direction;
        } else {
            self.envelope_level = if self.envelope_direction > 0 { 0 } else { 31 };
        }
    }

    fn channel_level(&self, channel: usize) -> u8 {
        let mixer = self.audio_registers[7];
        let tone_high = mixer & (1 << channel) != 0 || self.tone_output[channel];
        let noise_high = mixer & (1 << (channel + 3)) != 0 || self.noise_output;
        if !tone_high || !noise_high {
            return 0;
        }
        let volume = self.audio_registers[8 + channel];
        if volume & 0x10 != 0 {
            self.envelope_level
        } else if volume & 0x0f == 0 {
            0
        } else {
            (volume & 0x0f) * 2 + 1
        }
    }
}

impl Mapper for M069 {
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
        if (0x6000..0x8000).contains(&address) {
            return match self.window_mapping(address) {
                ReadMapping::PrgRam(offset) => WriteMapping::PrgRam(offset),
                _ => WriteMapping::Unmapped,
            };
        }
        match address {
            0x8000..=0x9fff => {
                self.command = value & 0x0f;
                WriteMapping::Handled
            }
            0xa000..=0xbfff => {
                self.write_parameter(value);
                WriteMapping::Handled
            }
            0xc000..=0xdfff => {
                self.audio_select = value;
                WriteMapping::Handled
            }
            0xe000..=0xffff => {
                self.write_audio_register(value);
                WriteMapping::Handled
            }
            _ => WriteMapping::Unmapped,
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

    fn clock_cpu(&mut self) {
        if self.irq_counter_enabled {
            self.irq_counter = self.irq_counter.wrapping_sub(1);
            if self.irq_counter == 0xffff && self.irq_enabled {
                self.irq_pending = true;
            }
        }
        self.audio_divider += 1;
        if self.audio_divider == 16 {
            self.audio_divider = 0;
            self.clock_audio_generators();
        }
    }

    fn irq_pending(&self) -> bool {
        self.irq_pending
    }

    fn expansion_audio_sample(&self) -> f32 {
        let mut sample = 0.0;
        for channel in 0..3 {
            let level = self.channel_level(channel);
            if level != 0 {
                // YM2149 levels are spaced approximately 1.5 dB apart.
                sample += 10.0_f32.powf((level as f32 - 31.0) * 1.5 / 20.0);
            }
        }
        sample / 3.0
    }

    fn reset(&mut self, kind: ResetKind) {
        if kind == ResetKind::PowerOn {
            self.irq_counter_enabled = false;
            self.irq_enabled = false;
            self.irq_pending = false;
            self.command = 0;
            self.chr_banks = [0; 8];
            self.prg_banks = [0; 3];
            self.window_bank = 0;
            self.mirroring = self.config.mirroring;
            self.irq_counter = 0;
            self.audio_select = 0;
            self.audio_registers = [0; 16];
            self.audio_divider = 0;
            self.tone_counter = [0; 3];
            self.tone_output = [false; 3];
            self.noise_counter = 0;
            self.noise_lfsr = 0x1ffff;
            self.noise_output = false;
            self.envelope_counter = 0;
            self.envelope_level = 31;
            self.envelope_direction = -1;
            self.envelope_holding = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TimingMode;

    fn config() -> CartridgeConfig {
        CartridgeConfig {
            mapper_id: 69,
            submapper: 0,
            prg_rom_len: 0x80000,
            chr_rom_len: 0x40000,
            prg_ram_len: 0x8000,
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

    fn command(mapper: &mut M069, command: u8, value: u8) {
        mapper.cpu_write(0x8000, command, None);
        mapper.cpu_write(0xa000, value, None);
    }

    fn audio(mapper: &mut M069, register: u8, value: u8) {
        mapper.cpu_write(0xc000, register, None);
        mapper.cpu_write(0xe000, value, None);
    }

    #[test]
    fn command_register_selects_chr_and_prg_banks() {
        let mut mapper = M069::new(config()).unwrap();
        command(&mut mapper, 5, 0xa5);
        command(&mut mapper, 9, 0x22);
        assert_eq!(mapper.ppu_peek(0x1400), ReadMapping::ChrRom(0xa5 * 0x400));
        assert_eq!(mapper.cpu_peek(0x8000), ReadMapping::PrgRom(0x22 * 0x2000));
        assert_eq!(mapper.cpu_peek(0xe000), ReadMapping::PrgRom(0x7e000));
    }

    #[test]
    fn six_thousand_window_switches_rom_ram_and_open_bus() {
        let mut mapper = M069::new(config()).unwrap();
        command(&mut mapper, 8, 5);
        assert_eq!(mapper.cpu_peek(0x6000), ReadMapping::PrgRom(5 * 0x2000));
        assert_eq!(mapper.cpu_write(0x6000, 0, None), WriteMapping::Unmapped);

        command(&mut mapper, 8, 0x40);
        assert_eq!(mapper.cpu_peek(0x6000), ReadMapping::OpenBus);

        command(&mut mapper, 8, 0xc2);
        assert_eq!(mapper.cpu_peek(0x6123), ReadMapping::PrgRam(0x4123));
        assert_eq!(
            mapper.cpu_write(0x6123, 0, None),
            WriteMapping::PrgRam(0x4123)
        );
    }

    #[test]
    fn irq_counter_runs_independently_and_asserts_on_underflow() {
        let mut mapper = M069::new(config()).unwrap();
        command(&mut mapper, 0xe, 1);
        command(&mut mapper, 0xf, 0);
        command(&mut mapper, 0xd, 0x81);
        mapper.clock_cpu();
        assert!(!mapper.irq_pending());
        mapper.clock_cpu();
        assert!(mapper.irq_pending());

        command(&mut mapper, 0xd, 0x80);
        assert!(!mapper.irq_pending());
        mapper.clock_cpu();
        assert!(!mapper.irq_pending());
        command(&mut mapper, 0xd, 0);
        let paused = mapper.irq_counter;
        mapper.clock_cpu();
        assert_eq!(mapper.irq_counter, paused);
    }

    #[test]
    fn mirroring_modes_match_fme7_encoding() {
        let mut mapper = M069::new(config()).unwrap();
        command(&mut mapper, 0xc, 3);
        assert_eq!(
            mapper.nametable_target(0x2000),
            NametableTarget::Ciram(0x400)
        );
        command(&mut mapper, 0xc, 0);
        assert_eq!(mapper.nametable_target(0x2800), NametableTarget::Ciram(0));
    }

    #[test]
    fn five_b_audio_honors_register_disable_and_mixer_gates() {
        let mut mapper = M069::new(config()).unwrap();
        audio(&mut mapper, 7, 0x3f); // bypass tone/noise generators
        audio(&mut mapper, 8, 0x0f);
        let full_level = mapper.expansion_audio_sample();
        assert!(full_level > 0.0);

        // A high nibble on the address latch disables the following write.
        audio(&mut mapper, 0x18, 0);
        assert_eq!(mapper.expansion_audio_sample(), full_level);
        audio(&mut mapper, 8, 0);
        assert_eq!(mapper.expansion_audio_sample(), 0.0);
    }

    #[test]
    fn five_b_tone_noise_and_envelope_advance_on_divided_clock() {
        let mut mapper = M069::new(config()).unwrap();
        audio(&mut mapper, 0, 1);
        audio(&mut mapper, 1, 0);
        audio(&mut mapper, 6, 1);
        audio(&mut mapper, 7, 0x3e); // channel A tone active, noise bypassed
        audio(&mut mapper, 8, 0x10); // envelope volume
        audio(&mut mapper, 11, 1);
        audio(&mut mapper, 12, 0);
        audio(&mut mapper, 13, 0x0c); // ascending, repeating envelope

        let initial = mapper.envelope_level;
        for _ in 0..15 {
            mapper.clock_cpu();
        }
        assert_eq!(mapper.envelope_level, initial);
        mapper.clock_cpu();
        assert_eq!(mapper.envelope_level, initial + 1);
        assert!(mapper.tone_output[0]);
        assert_ne!(mapper.noise_lfsr, 0x1ffff);
    }
}
