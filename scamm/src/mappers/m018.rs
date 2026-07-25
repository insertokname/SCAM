use crate::{
    CartridgeConfig, Mapper, MapperError, Mirroring, NametableTarget, ReadMapping, ResetKind,
    WriteMapping,
};

use super::{bank_offset, nametable_target};

#[derive(Debug, Clone)]
pub struct M018Resources {
    /// Decoded, normalized PCM samples indexed by the six-bit D7756 sample
    /// select input.
    pub d7756_samples: Vec<Vec<f32>>,
    /// CPU clocks for which each decoded PCM value is held.
    pub clocks_per_sample: u32,
}

impl Default for M018Resources {
    fn default() -> Self {
        Self {
            d7756_samples: Vec::new(),
            clocks_per_sample: 1,
        }
    }
}

/// Jaleco SS88006 (iNES mapper 18), optionally with a D7756 sample player.
#[derive(Debug)]
pub struct M018 {
    config: CartridgeConfig,
    prg_banks: [u8; 3],
    chr_banks: [u8; 8],
    prg_ram_control: u8,
    mirroring: Mirroring,
    irq_reload: u16,
    irq_counter: u16,
    irq_mask: u16,
    irq_enabled: bool,
    irq_pending: bool,
    resources: M018Resources,
    sample_select: u8,
    sample_reset: bool,
    sample_start: bool,
    reset_cycles: u8,
    ready_delay: u8,
    playing_sample: Option<usize>,
    sample_position: usize,
    sample_clock: u32,
    sample_output: f32,
}

impl M018 {
    pub fn new(config: CartridgeConfig) -> Result<Self, MapperError> {
        Self::new_with_resources(config, M018Resources::default())
    }

    pub fn new_with_resources(
        config: CartridgeConfig,
        resources: M018Resources,
    ) -> Result<Self, MapperError> {
        if config.submapper != 0 {
            return Err(MapperError::Unsupported {
                mapper_id: 18,
                submapper: config.submapper,
            });
        }
        if config.prg_rom_len < 0x8000
            || config.prg_rom_len > 0x80000
            || !config.prg_rom_len.is_multiple_of(0x2000)
        {
            return Err(MapperError::InvalidConfiguration {
                mapper_id: 18,
                reason: "SS88006 PRG ROM must be 32-512 KiB in 8 KiB units",
            });
        }
        if config.chr_rom_len < 0x2000
            || config.chr_rom_len > 0x40000
            || !config.chr_rom_len.is_multiple_of(0x0400)
            || config.total_chr_ram_len() != 0
        {
            return Err(MapperError::InvalidConfiguration {
                mapper_id: 18,
                reason: "SS88006 requires 8-256 KiB CHR ROM in 1 KiB units",
            });
        }
        if !matches!(config.total_prg_ram_len(), 0 | 0x2000) {
            return Err(MapperError::InvalidConfiguration {
                mapper_id: 18,
                reason: "SS88006 supports either no PRG RAM or one 8 KiB PRG RAM device",
            });
        }

        let mirroring = config.mirroring;
        Ok(Self {
            config,
            prg_banks: [0; 3],
            chr_banks: [0; 8],
            prg_ram_control: 0,
            mirroring,
            irq_reload: 0,
            irq_counter: 0,
            irq_mask: 0xffff,
            irq_enabled: false,
            irq_pending: false,
            resources,
            sample_select: 0,
            sample_reset: false,
            sample_start: false,
            reset_cycles: 0,
            ready_delay: 0,
            playing_sample: None,
            sample_position: 0,
            sample_clock: 0,
            sample_output: 0.0,
        })
    }

    fn prg_ram_read_mapping(&self, address: u16) -> Option<ReadMapping> {
        ((0x6000..0x8000).contains(&address)
            && self.config.total_prg_ram_len() != 0
            && self.prg_ram_control & 1 != 0)
            .then(|| {
                ReadMapping::PrgRam((address as usize - 0x6000) % self.config.total_prg_ram_len())
            })
    }

    fn cpu_mapping(&self, address: u16) -> ReadMapping {
        if let Some(mapping) = self.prg_ram_read_mapping(address) {
            return mapping;
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

    fn update_nibble(register: &mut u8, high: bool, value: u8) {
        if high {
            *register = (*register & 0x0f) | ((value & 0x0f) << 4);
        } else {
            *register = (*register & 0xf0) | (value & 0x0f);
        }
    }

    fn write_register(&mut self, address: u16, value: u8) {
        let register = address & 0xf003;
        match register {
            0x8000..=0x8003 => {
                let slot = usize::from(address & 2 != 0);
                Self::update_nibble(&mut self.prg_banks[slot], address & 1 != 0, value);
                // PRG A16-A18 are the only high bank lines connected.
                self.prg_banks[slot] &= 0x3f;
            }
            0x9000 | 0x9001 => {
                Self::update_nibble(&mut self.prg_banks[2], address & 1 != 0, value);
                self.prg_banks[2] &= 0x3f;
            }
            0x9002 => self.prg_ram_control = value & 3,
            0xa000..=0xd003 => {
                let group = ((address as usize >> 12) - 0x0a) * 2;
                let slot = group + usize::from(address & 2 != 0);
                Self::update_nibble(&mut self.chr_banks[slot], address & 1 != 0, value);
            }
            0xe000..=0xe003 => {
                let shift = (address as u32 & 3) * 4;
                self.irq_reload =
                    (self.irq_reload & !(0x000f << shift)) | (((value & 0x0f) as u16) << shift);
            }
            0xf000 => {
                self.irq_counter = self.irq_reload;
                self.irq_pending = false;
            }
            0xf001 => {
                self.irq_enabled = value & 1 != 0;
                self.irq_mask = if value & 0x08 != 0 {
                    0x000f
                } else if value & 0x04 != 0 {
                    0x00ff
                } else if value & 0x02 != 0 {
                    0x0fff
                } else {
                    0xffff
                };
                self.irq_pending = false;
            }
            0xf002 => {
                self.mirroring = match value & 3 {
                    0 => Mirroring::Horizontal,
                    1 => Mirroring::Vertical,
                    2 => Mirroring::OneScreenLower,
                    _ => Mirroring::OneScreenUpper,
                };
            }
            0xf003 => self.write_sample_control(value),
            _ => {}
        }
    }

    fn write_sample_control(&mut self, value: u8) {
        let reset = value & 1 != 0;
        let start = value & 2 != 0;
        self.sample_select = value >> 2;

        if reset && !self.sample_reset {
            self.reset_cycles = 0;
        } else if !reset && self.sample_reset {
            self.ready_delay = 23;
        }
        if start && !self.sample_start && !reset && self.ready_delay == 0 {
            let selected = self.sample_select as usize;
            if self.resources.d7756_samples.get(selected).is_some() {
                self.playing_sample = Some(selected);
                self.sample_position = 0;
                self.sample_clock = 0;
                self.sample_output = 0.0;
            }
        }
        self.sample_reset = reset;
        self.sample_start = start;
    }

    fn clock_sample_player(&mut self) {
        if self.sample_reset {
            self.reset_cycles = self.reset_cycles.saturating_add(1);
            if self.reset_cycles >= 34 {
                self.playing_sample = None;
                self.sample_output = 0.0;
            }
            return;
        }
        self.ready_delay = self.ready_delay.saturating_sub(1);
        let Some(sample_index) = self.playing_sample else {
            return;
        };
        self.sample_clock += 1;
        if self.sample_clock < self.resources.clocks_per_sample.max(1) {
            return;
        }
        self.sample_clock = 0;
        let sample = &self.resources.d7756_samples[sample_index];
        if let Some(value) = sample.get(self.sample_position) {
            self.sample_output = value.clamp(-1.0, 1.0);
            self.sample_position += 1;
        } else {
            self.playing_sample = None;
            self.sample_output = 0.0;
        }
    }
}

impl Mapper for M018 {
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
            if let Some(ReadMapping::PrgRam(offset)) = self.prg_ram_read_mapping(address)
                && self.prg_ram_control & 2 != 0
            {
                return WriteMapping::PrgRam(offset);
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

    fn ppu_write(&mut self, _: u16, _: u8) -> WriteMapping {
        WriteMapping::Unmapped
    }

    fn nametable_target(&self, address: u16) -> NametableTarget {
        nametable_target(self.mirroring, address)
    }

    fn clock_cpu(&mut self) {
        if self.irq_enabled {
            let low = self.irq_counter & self.irq_mask;
            let next = low.wrapping_sub(1) & self.irq_mask;
            self.irq_counter = (self.irq_counter & !self.irq_mask) | next;
            if next == self.irq_mask {
                self.irq_pending = true;
            }
        }
        self.clock_sample_player();
    }

    fn irq_pending(&self) -> bool {
        self.irq_pending
    }

    fn expansion_audio_sample(&self) -> f32 {
        self.sample_output
    }

    fn reset(&mut self, kind: ResetKind) {
        if kind == ResetKind::PowerOn {
            self.prg_banks = [0; 3];
            self.chr_banks = [0; 8];
            self.prg_ram_control = 0;
            self.mirroring = self.config.mirroring;
            self.irq_enabled = false;
            self.irq_pending = false;
            self.irq_reload = 0;
            self.irq_counter = 0;
            self.irq_mask = 0xffff;
            self.sample_select = 0;
            self.sample_reset = false;
            self.sample_start = false;
            self.reset_cycles = 0;
            self.ready_delay = 0;
            self.playing_sample = None;
            self.sample_position = 0;
            self.sample_clock = 0;
            self.sample_output = 0.0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TimingMode;

    fn config() -> CartridgeConfig {
        CartridgeConfig {
            mapper_id: 18,
            submapper: 0,
            prg_rom_len: 0x80000,
            chr_rom_len: 0x40000,
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

    #[test]
    fn split_nibbles_select_prg_and_chr_banks() {
        let mut mapper = M018::new(config()).unwrap();
        mapper.cpu_write(0x8000, 0x0a, None);
        mapper.cpu_write(0x8001, 0x03, None);
        mapper.cpu_write(0xc002, 0x05, None);
        mapper.cpu_write(0xc003, 0x0a, None);
        assert_eq!(mapper.cpu_peek(0x8000), ReadMapping::PrgRom(0x3a * 0x2000));
        assert_eq!(mapper.ppu_peek(0x1400), ReadMapping::ChrRom(0xa5 * 0x400));
        assert_eq!(mapper.cpu_peek(0xe000), ReadMapping::PrgRom(0x7e000));
    }

    #[test]
    fn prg_ram_enable_and_write_protect_are_independent() {
        let mut mapper = M018::new(config()).unwrap();
        assert_eq!(mapper.cpu_peek(0x6000), ReadMapping::OpenBus);
        mapper.cpu_write(0x9002, 1, None);
        assert_eq!(mapper.cpu_peek(0x6123), ReadMapping::PrgRam(0x123));
        assert_eq!(mapper.cpu_write(0x6123, 0, None), WriteMapping::Unmapped);
        mapper.cpu_write(0x9002, 3, None);
        assert_eq!(
            mapper.cpu_write(0x6123, 0, None),
            WriteMapping::PrgRam(0x123)
        );
    }

    #[test]
    fn irq_asserts_on_selected_counter_underflow_and_preserves_high_bits() {
        let mut mapper = M018::new(config()).unwrap();
        // Reload $1230, then select a 4-bit counter.
        for (address, value) in [(0xe000, 0), (0xe001, 3), (0xe002, 2), (0xe003, 1)] {
            mapper.cpu_write(address, value, None);
        }
        mapper.cpu_write(0xf000, 0, None);
        mapper.cpu_write(0xf001, 0x09, None);
        mapper.clock_cpu();
        assert!(mapper.irq_pending());
        assert_eq!(mapper.irq_counter, 0x123f);
        mapper.cpu_write(0xfffd, 0x09, None);
        assert!(!mapper.irq_pending());
    }

    #[test]
    fn mirrored_register_addresses_use_only_connected_lines() {
        let mut mapper = M018::new(config()).unwrap();
        mapper.cpu_write(0x8ffd, 2, None); // mirrors $8001
        assert_eq!(mapper.cpu_peek(0x8000), ReadMapping::PrgRom(0x20 * 0x2000));
        mapper.cpu_write(0xfffe, 1, None); // mirrors $F002
        assert_eq!(mapper.nametable_target(0x2800), NametableTarget::Ciram(0));
    }

    #[test]
    fn d7756_reset_ready_start_and_sample_hold_timing() {
        let resources = M018Resources {
            d7756_samples: vec![Vec::new(), vec![0.25, -0.5]],
            clocks_per_sample: 2,
        };
        let mut mapper = M018::new_with_resources(config(), resources).unwrap();

        mapper.cpu_write(0xf003, 0x05, None); // sample 1, reset asserted
        for _ in 0..34 {
            mapper.clock_cpu();
        }
        mapper.cpu_write(0xf003, 0x04, None); // reset released
        for _ in 0..22 {
            mapper.clock_cpu();
        }
        mapper.cpu_write(0xf003, 0x06, None); // too early: ignored start edge
        assert_eq!(mapper.playing_sample, None);

        mapper.cpu_write(0xf003, 0x04, None);
        mapper.clock_cpu();
        mapper.cpu_write(0xf003, 0x06, None);
        assert_eq!(mapper.playing_sample, Some(1));
        mapper.clock_cpu();
        assert_eq!(mapper.expansion_audio_sample(), 0.0);
        mapper.clock_cpu();
        assert_eq!(mapper.expansion_audio_sample(), 0.25);
        mapper.clock_cpu();
        assert_eq!(mapper.expansion_audio_sample(), 0.25);
        mapper.clock_cpu();
        assert_eq!(mapper.expansion_audio_sample(), -0.5);

        mapper.cpu_write(0xf003, 0x05, None);
        for _ in 0..33 {
            mapper.clock_cpu();
        }
        assert_eq!(mapper.expansion_audio_sample(), -0.5);
        mapper.clock_cpu();
        assert_eq!(mapper.expansion_audio_sample(), 0.0);
    }

    #[test]
    fn resource_free_sample_player_is_silent_and_soft_reset_preserves_banks() {
        let mut mapper = M018::new(config()).unwrap();
        mapper.cpu_write(0x8000, 3, None);
        mapper.cpu_write(0xf003, 0xfe, None);
        for _ in 0..100 {
            mapper.clock_cpu();
        }
        mapper.reset(ResetKind::ResetButton);
        assert_eq!(mapper.expansion_audio_sample(), 0.0);
        assert_eq!(mapper.cpu_peek(0x8000), ReadMapping::PrgRom(3 * 0x2000));
    }
}
