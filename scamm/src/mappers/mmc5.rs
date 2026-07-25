use crate::{
    BusValue, CartridgeConfig, Mapper, MapperError, NametableTarget, PpuFetch, PpuFetchKind,
    ReadMapping, ResetKind, WriteMapping,
};

use super::bank_offset;

const LENGTH_TABLE: [u8; 32] = [
    10, 254, 20, 2, 40, 4, 80, 6, 160, 8, 60, 10, 14, 12, 26, 14, 12, 16, 24, 18, 48, 20, 96, 22,
    192, 24, 72, 26, 16, 28, 32, 30,
];

#[derive(Debug, Clone, Copy)]
struct Mmc5Pulse {
    control: u8,
    timer: u16,
    timer_counter: u16,
    sequence: u8,
    length: u8,
    enabled: bool,
    envelope_start: bool,
    envelope_divider: u8,
    envelope_decay: u8,
}

impl Mmc5Pulse {
    const fn new() -> Self {
        Self {
            control: 0,
            timer: 0,
            timer_counter: 0,
            sequence: 0,
            length: 0,
            enabled: false,
            envelope_start: false,
            envelope_divider: 0,
            envelope_decay: 0,
        }
    }

    fn write(&mut self, register: u8, value: u8) {
        match register {
            0 => self.control = value,
            2 => self.timer = (self.timer & 0x0700) | u16::from(value),
            3 => {
                self.timer = (self.timer & 0x00ff) | (u16::from(value & 7) << 8);
                if self.enabled {
                    self.length = LENGTH_TABLE[(value >> 3) as usize];
                }
                self.sequence = 0;
                self.envelope_start = true;
            }
            _ => {}
        }
    }

    fn clock_timer(&mut self) {
        if self.timer_counter == 0 {
            self.timer_counter = self.timer;
            self.sequence = (self.sequence + 1) & 7;
        } else {
            self.timer_counter -= 1;
        }
    }

    fn clock_frame(&mut self) {
        if self.control & 0x20 == 0 && self.length != 0 {
            self.length -= 1;
        }
        if self.envelope_start {
            self.envelope_start = false;
            self.envelope_decay = 15;
            self.envelope_divider = self.control & 0x0f;
        } else if self.envelope_divider == 0 {
            self.envelope_divider = self.control & 0x0f;
            if self.envelope_decay != 0 {
                self.envelope_decay -= 1;
            } else if self.control & 0x20 != 0 {
                self.envelope_decay = 15;
            }
        } else {
            self.envelope_divider -= 1;
        }
    }

    fn sample(&self) -> u8 {
        const DUTY: [[bool; 8]; 4] = [
            [false, true, false, false, false, false, false, false],
            [false, true, true, false, false, false, false, false],
            [false, true, true, true, true, false, false, false],
            [true, false, false, true, true, true, true, true],
        ];
        if !self.enabled
            || self.length == 0
            || !DUTY[(self.control >> 6) as usize][self.sequence as usize]
        {
            0
        } else if self.control & 0x10 != 0 {
            self.control & 0x0f
        } else {
            self.envelope_decay
        }
    }
}

/// Nintendo MMC5 / ExROM (mapper 5).
///
/// The implementation models CPU-visible configuration, PRG/CHR banking,
/// ExRAM and nametable substitutions, scanline and timer IRQs, MMC5A pins,
/// multiplication, pulse audio, and read/write-mode PCM.
#[derive(Debug)]
pub struct M005 {
    config: CartridgeConfig,
    prg_mode: u8,
    chr_mode: u8,
    ram_protect_1: u8,
    ram_protect_2: u8,
    exram_mode: u8,
    nametable_map: u8,
    fill_tile: u8,
    fill_color: u8,
    prg_banks: [u8; 5],
    chr_banks: [u16; 12],
    chr_upper: u16,
    last_chr_set_background: bool,
    exram: [u8; 0x400],
    last_fetch_kind: PpuFetchKind,
    rendering: bool,
    sprite_16: bool,
    extended_attribute: u8,
    split_control: u8,
    split_scroll: u8,
    split_bank: u8,
    split_tile: u8,
    split_active: bool,
    split_exram_offset: usize,
    irq_compare: u8,
    irq_scanline: u8,
    irq_enabled: bool,
    irq_pending: bool,
    in_frame: bool,
    last_nt_address: u16,
    matching_nt_reads: u8,
    scanline_detection_pending: bool,
    saw_ppu_this_cpu_cycle: bool,
    idle_cpu_cycles: u8,
    multiplicand: u8,
    multiplier: u8,
    pulses: [Mmc5Pulse; 2],
    pulse_clock_half: bool,
    audio_frame_divider: u16,
    pcm_read_mode: bool,
    pcm_irq_enabled: bool,
    pcm_irq_pending: bool,
    pcm_value: u8,
    pin_config: u8,
    pin_output: u8,
    pin_inputs: [bool; 2],
    pin_92: bool,
    pin_read_strobe: bool,
    pin_write_strobe: bool,
    timer_counter: u16,
    timer_running: bool,
    timer_irq_pending: bool,
}

impl M005 {
    pub fn new(config: CartridgeConfig) -> Result<Self, MapperError> {
        if config.submapper != 0 {
            return Err(MapperError::Unsupported {
                mapper_id: 5,
                submapper: config.submapper,
            });
        }
        if config.prg_rom_len < 0x8000 || !config.prg_rom_len.is_multiple_of(0x2000) {
            return Err(MapperError::InvalidConfiguration {
                mapper_id: 5,
                reason: "MMC5 requires PRG ROM in 8 KiB banks",
            });
        }
        if config.prg_rom_len > 0x100000 {
            return Err(MapperError::InvalidConfiguration {
                mapper_id: 5,
                reason: "MMC5 exposes at most 1 MiB of PRG ROM",
            });
        }
        if !matches!(
            config.total_prg_ram_len(),
            0 | 0x2000 | 0x4000 | 0x8000 | 0x10000
        ) || !config.prg_ram_len.is_multiple_of(0x2000)
            || !config.prg_nvram_len.is_multiple_of(0x2000)
        {
            return Err(MapperError::InvalidConfiguration {
                mapper_id: 5,
                reason: "MMC5 PRG RAM wiring supports 0, 8, 16, 32, or 64 KiB in whole 8 KiB chips/banks",
            });
        }
        let chr_ram_len = config.total_chr_ram_len();
        let chr_len = config.chr_rom_len.max(chr_ram_len);
        if config.chr_rom_len != 0 && chr_ram_len != 0
            || chr_len == 0
            || chr_len > 0x100000
            || !chr_len.is_multiple_of(0x0400)
        {
            return Err(MapperError::InvalidConfiguration {
                mapper_id: 5,
                reason: "MMC5 requires exactly one 1 KiB-granular CHR ROM or RAM device up to 1 MiB",
            });
        }
        Ok(Self {
            config,
            prg_mode: 3,
            chr_mode: 3,
            ram_protect_1: 1, // Wiki: $5102 reset value is 01
            ram_protect_2: 2, // Wiki: $5103 reset value is 10
            exram_mode: 3,    // Wiki: $5104 reset value is 11 (read-only mode)
            nametable_map: 0,
            fill_tile: 0,
            fill_color: 0,
            prg_banks: [0, 0, 0, 0, 0xff],
            chr_banks: [0; 12],
            chr_upper: 0,
            last_chr_set_background: false,
            exram: [0; 0x400],
            last_fetch_kind: PpuFetchKind::Other,
            rendering: false,
            sprite_16: false,
            extended_attribute: 0,
            split_control: 0,
            split_scroll: 0,
            split_bank: 0,
            split_tile: 0,
            split_active: false,
            split_exram_offset: 0,
            irq_compare: 0,
            irq_scanline: 0,
            irq_enabled: false,
            irq_pending: false,
            in_frame: false,
            last_nt_address: 0,
            matching_nt_reads: 0,
            scanline_detection_pending: false,
            saw_ppu_this_cpu_cycle: false,
            idle_cpu_cycles: 0,
            // MMC5A power analysis reports both multiplier operands as $FF.
            multiplicand: 0xff,
            multiplier: 0xff,
            pulses: [Mmc5Pulse::new(); 2],
            pulse_clock_half: false,
            audio_frame_divider: 0,
            pcm_read_mode: false,
            pcm_irq_enabled: false,
            pcm_irq_pending: false,
            pcm_value: 0,
            pin_config: 0xc0,
            pin_output: 0,
            pin_inputs: [true; 2],
            pin_92: true,
            pin_read_strobe: false,
            pin_write_strobe: false,
            timer_counter: 0,
            timer_running: false,
            timer_irq_pending: false,
        })
    }

    fn prg_register_for_slot(&self, slot: usize) -> (u8, usize) {
        match self.prg_mode {
            0 => (self.prg_banks[4], slot),
            1 => {
                if slot < 2 {
                    (self.prg_banks[2], slot)
                } else {
                    (self.prg_banks[4], slot - 2)
                }
            }
            2 => match slot {
                0 | 1 => (self.prg_banks[2], slot),
                2 => (self.prg_banks[3], 0),
                _ => (self.prg_banks[4], 0),
            },
            _ => (self.prg_banks[slot + 1], 0),
        }
    }

    fn prg_ram_offset(&self, bank: usize, inner: usize) -> Option<usize> {
        let volatile = self.config.prg_ram_len;
        let nonvolatile = self.config.prg_nvram_len;
        let total = volatile + nonvolatile;
        let base = match total {
            0 => return None,
            0x2000 => (bank < 4).then_some(0)?,
            0x4000 if volatile == 0x2000 && nonvolatile == 0x2000 => {
                if bank < 4 {
                    volatile
                } else {
                    0
                }
            }
            0x4000 => {
                if bank < 4 {
                    0
                } else {
                    0x2000
                }
            }
            0x8000 => {
                if bank >= 4 {
                    return None;
                }
                bank * 0x2000
            }
            0x10000 => bank * 0x2000,
            _ => return None,
        };
        Some(base + (inner & 0x1fff))
    }

    fn prg_mapping(&self, address: u16) -> ReadMapping {
        if (0x6000..0x8000).contains(&address) {
            let bank = (self.prg_banks[0] & 0x07) as usize;
            return self
                .prg_ram_offset(bank, address as usize)
                .map_or(ReadMapping::OpenBus, ReadMapping::PrgRam);
        }
        if address < 0x8000 {
            return ReadMapping::OpenBus;
        }
        let slot = (address as usize - 0x8000) / 0x2000;
        let (register, slot_in_bank) = self.prg_register_for_slot(slot);
        let is_last = slot == 3;
        let use_rom = is_last || register & 0x80 != 0;
        let align_mask = match self.prg_mode {
            0 => !3usize,
            1 => !1usize,
            2 if slot < 2 => !1usize,
            _ => !0usize,
        };
        let bank = ((register & 0x7f) as usize & align_mask) + slot_in_bank;
        if use_rom {
            ReadMapping::PrgRom(bank_offset(
                bank,
                0x2000,
                self.config.prg_rom_len,
                address as usize & 0x1fff,
            ))
        } else if let Some(offset) = self.prg_ram_offset(bank, address as usize) {
            ReadMapping::PrgRam(offset)
        } else {
            ReadMapping::OpenBus
        }
    }

    fn chr_mapping(&self, address: u16) -> ReadMapping {
        if address >= 0x2000 {
            return ReadMapping::OpenBus;
        }
        let slot = address as usize / 0x0400;
        let extended = self.rendering
            && self.last_fetch_kind == PpuFetchKind::BackgroundPattern
            && self.exram_mode == 1
            && self.pin_92
            && !self.split_active;
        let bank = if self.split_active
            && self.last_fetch_kind == PpuFetchKind::BackgroundPattern
            && self.rendering
        {
            (self.split_bank as usize * 4) + ((address as usize & 0x0fff) / 0x0400)
        } else if extended {
            ((((self.chr_upper >> 8) as usize) << 6) | (self.extended_attribute as usize & 0x3f))
                * 4
                + ((address as usize & 0x0fff) / 0x0400)
        } else {
            let use_background = self.pin_92
                && self.rendering
                && self.sprite_16
                && self.last_fetch_kind == PpuFetchKind::BackgroundPattern
                || self.last_fetch_kind == PpuFetchKind::Cpu && self.last_chr_set_background;
            let logical_slot = if use_background { slot & 3 } else { slot };
            let inner_slot = if use_background && self.chr_mode != 0 {
                slot & 3
            } else {
                slot
            };
            let base = if use_background { 8 } else { 0 };
            let (register_index, slots_per_bank) = match self.chr_mode {
                0 => (base + if use_background { 3 } else { 7 }, 8),
                1 => (base + if use_background || slot < 4 { 3 } else { 7 }, 4),
                2 => (base + [1, 1, 3, 3, 5, 5, 7, 7][logical_slot], 2),
                _ => (base + logical_slot, 1),
            };
            self.chr_banks[register_index] as usize + inner_slot % slots_per_bank
        };
        let len = if self.config.chr_rom_len == 0 {
            self.config.total_chr_ram_len()
        } else {
            self.config.chr_rom_len
        };
        let offset = bank_offset(bank, 0x0400, len, address as usize & 0x03ff);
        if self.config.chr_rom_len == 0 {
            ReadMapping::ChrRam(offset)
        } else {
            ReadMapping::ChrRom(offset)
        }
    }

    fn capture_chr_bank(&self, value: u8) -> u16 {
        // Bit 0 always captures CPU D0 even in the larger-bank modes. It is a
        // don't-care while that mode remains selected, but becomes observable
        // if software changes CHR mode without rewriting the bank register.
        match self.chr_mode {
            0 => ((u16::from(value) << 3) & 0x03ff) | u16::from(value & 1),
            1 => ((u16::from(value) << 2) & 0x03ff) | u16::from(value & 1),
            2 => ((self.chr_upper & 0x0100) << 1) | (u16::from(value) << 1) | u16::from(value & 1),
            _ => self.chr_upper | u16::from(value),
        }
    }

    fn ram_writable(&self) -> bool {
        self.ram_protect_1 & 3 == 2 && self.ram_protect_2 & 3 == 1
    }

    fn pin_level(&self, pin: usize) -> bool {
        let function_bit = 1 << pin;
        if self.pin_config & function_bit != 0 {
            return if pin == 0 {
                !self.pin_read_strobe
            } else {
                !self.pin_write_strobe
            };
        }
        let direction_input = self.pin_config & (0x40 << pin) != 0;
        if direction_input {
            self.pin_inputs[pin]
        } else {
            self.pin_output & (0x40 << pin) != 0
        }
    }

    fn write_pcm(&mut self, value: u8) {
        if value == 0 {
            self.pcm_irq_pending = true;
        } else {
            self.pcm_irq_pending = false;
            self.pcm_value = value;
        }
    }

    fn cpu_register_read(&mut self, address: u16, peek: bool) -> Option<ReadMapping> {
        let value = match address {
            0x5010 => {
                let value = (u8::from(self.pcm_irq_pending && self.pcm_irq_enabled) << 7) | 1;
                if !peek {
                    self.pcm_irq_pending = false;
                }
                value
            }
            0x5015 => {
                u8::from(self.pulses[0].length != 0) | (u8::from(self.pulses[1].length != 0) << 1)
            }
            0x5204 => {
                let value = (u8::from(self.irq_pending) << 7) | (u8::from(self.in_frame) << 6);
                if !peek {
                    self.irq_pending = false;
                }
                value
            }
            0x5205 => self.multiplicand.wrapping_mul(self.multiplier),
            0x5206 => ((self.multiplicand as u16 * self.multiplier as u16) >> 8) as u8,
            0x5208 => (u8::from(self.pin_level(0)) << 6) | (u8::from(self.pin_level(1)) << 7),
            0x5209 => {
                let value = u8::from(self.timer_irq_pending) << 7;
                if !peek {
                    self.timer_irq_pending = false;
                }
                value
            }
            0x5800..=0x5bff => {
                if !peek {
                    self.pin_read_strobe = true;
                }
                return Some(ReadMapping::OpenBus);
            }
            0x5c00..=0x5fff if self.exram_mode >= 2 => self.exram[address as usize & 0x03ff],
            0x5c00..=0x5fff => return Some(ReadMapping::OpenBus),
            _ => return None,
        };
        Some(ReadMapping::Value(BusValue::full(value)))
    }

    fn cpu_register_write(&mut self, address: u16, value: u8) -> bool {
        match address {
            0x5000..=0x5003 => self.pulses[0].write((address & 3) as u8, value),
            0x5004..=0x5007 => self.pulses[1].write((address & 3) as u8, value),
            0x5010 => {
                self.pcm_read_mode = value & 1 != 0;
                self.pcm_irq_enabled = value & 0x80 != 0;
            }
            0x5011 if !self.pcm_read_mode => self.write_pcm(value),
            0x5011 => {}
            0x5015 => {
                for channel in 0..2 {
                    self.pulses[channel].enabled = value & (1 << channel) != 0;
                    if !self.pulses[channel].enabled {
                        self.pulses[channel].length = 0;
                    }
                }
            }
            0x5100 => self.prg_mode = value & 3,
            0x5101 => self.chr_mode = value & 3,
            0x5102 => self.ram_protect_1 = value & 3,
            0x5103 => self.ram_protect_2 = value & 3,
            0x5104 => self.exram_mode = value & 3,
            0x5105 => self.nametable_map = value,
            0x5106 => self.fill_tile = value,
            0x5107 => self.fill_color = value & 3,
            0x5113..=0x5117 => self.prg_banks[(address - 0x5113) as usize] = value,
            0x5120..=0x512b => {
                self.chr_banks[(address - 0x5120) as usize] = self.capture_chr_bank(value);
                self.last_chr_set_background = address >= 0x5128;
            }
            0x5130 => self.chr_upper = ((value & 3) as u16) << 8,
            0x5200 => self.split_control = value,
            0x5201 => self.split_scroll = value,
            0x5202 => self.split_bank = value,
            0x5203 => self.irq_compare = value,
            0x5204 => {
                self.irq_enabled = value & 0x80 != 0;
            }
            0x5205 => self.multiplicand = value,
            0x5206 => self.multiplier = value,
            0x5207 => self.pin_config = value & 0xc3,
            0x5208 => self.pin_output = value & 0xc0,
            0x5209 => {
                self.timer_counter = (self.timer_counter & 0xff00) | u16::from(value);
                self.timer_running = self.timer_counter != 0;
                self.timer_irq_pending = false;
            }
            0x520a => {
                self.timer_counter = (self.timer_counter & 0x00ff) | (u16::from(value) << 8);
            }
            0x5800..=0x5bff => self.pin_write_strobe = true,
            0x5c00..=0x5fff if self.exram_mode == 2 || self.exram_mode < 2 && self.rendering => {
                self.exram[address as usize & 0x03ff] = value
            }
            0x5c00..=0x5fff => {}
            _ => return false,
        }
        true
    }
}

impl Mapper for M005 {
    fn config(&self) -> &CartridgeConfig {
        &self.config
    }
    fn cpu_read(&mut self, address: u16) -> ReadMapping {
        self.cpu_register_read(address, false)
            .unwrap_or_else(|| self.prg_mapping(address))
    }
    fn cpu_peek(&self, address: u16) -> ReadMapping {
        let value = match address {
            0x5010 => Some((u8::from(self.pcm_irq_pending && self.pcm_irq_enabled) << 7) | 1),
            0x5015 => Some(
                u8::from(self.pulses[0].length != 0) | (u8::from(self.pulses[1].length != 0) << 1),
            ),
            0x5204 => Some((u8::from(self.irq_pending) << 7) | (u8::from(self.in_frame) << 6)),
            0x5205 => Some(self.multiplicand.wrapping_mul(self.multiplier)),
            0x5206 => Some(((self.multiplicand as u16 * self.multiplier as u16) >> 8) as u8),
            0x5208 => Some((u8::from(self.pin_level(0)) << 6) | (u8::from(self.pin_level(1)) << 7)),
            0x5209 => Some(u8::from(self.timer_irq_pending) << 7),
            0x5c00..=0x5fff if self.exram_mode >= 2 => Some(self.exram[address as usize & 0x03ff]),
            _ => None,
        };
        value
            .map(|v| ReadMapping::Value(BusValue::full(v)))
            .unwrap_or_else(|| self.prg_mapping(address))
    }
    fn cpu_read_result(&mut self, address: u16, value: u8) {
        if self.pcm_read_mode && (0x8000..=0xbfff).contains(&address) {
            self.write_pcm(value);
        }
        if matches!(address, 0xfffa | 0xfffb) {
            self.in_frame = false;
            self.matching_nt_reads = 0;
            self.scanline_detection_pending = false;
        }
    }
    fn cpu_write(&mut self, address: u16, value: u8, _: Option<u8>) -> WriteMapping {
        if self.cpu_register_write(address, value) {
            return WriteMapping::Handled;
        }
        if let ReadMapping::PrgRam(offset) = self.prg_mapping(address) {
            if self.ram_writable() {
                WriteMapping::PrgRam(offset)
            } else {
                WriteMapping::Handled
            }
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
        if self.split_active && self.rendering {
            if self.last_fetch_kind == PpuFetchKind::BackgroundNametable {
                return NametableTarget::MapperRam(self.split_exram_offset & 0x03ff);
            }
            if self.last_fetch_kind == PpuFetchKind::BackgroundAttribute {
                let row = self.split_exram_offset / 32;
                let column = self.split_exram_offset % 32;
                return NametableTarget::MapperRam(0x03c0 + row / 4 * 8 + column / 4);
            }
        }
        if self.exram_mode == 1
            && self.pin_92
            && self.last_fetch_kind == PpuFetchKind::BackgroundAttribute
        {
            return NametableTarget::Fill {
                tile: 0,
                attribute: (self.extended_attribute >> 6) * 0x55,
            };
        }
        let table = ((address.wrapping_sub(0x2000) >> 10) & 3) as usize;
        match (self.nametable_map >> (table * 2)) & 3 {
            0 => NametableTarget::Ciram(address as usize & 0x03ff),
            1 => NametableTarget::Ciram(0x0400 | address as usize & 0x03ff),
            2 => NametableTarget::MapperRam(address as usize & 0x03ff),
            _ => NametableTarget::Fill {
                tile: self.fill_tile,
                attribute: self.fill_color * 0x55,
            },
        }
    }
    fn mapper_ram_read(&self, offset: usize) -> BusValue {
        BusValue::full(if self.exram_mode >= 2 {
            0
        } else {
            self.exram[offset & 0x03ff]
        })
    }
    fn mapper_ram_write(&mut self, offset: usize, value: u8) {
        if self.exram_mode < 2 {
            self.exram[offset & 0x03ff] = value;
        }
    }
    fn notify_ppu_fetch(&mut self, fetch: PpuFetch) {
        self.last_fetch_kind = fetch.kind;
        self.rendering = fetch.rendering;
        self.sprite_16 = fetch.sprite_16;
        if !fetch.is_read {
            return;
        }
        let address = fetch.address;
        self.saw_ppu_this_cpu_cycle = true;
        if !self.pin_92 {
            self.in_frame = false;
            self.irq_scanline = 0;
            self.matching_nt_reads = 0;
            self.scanline_detection_pending = false;
            self.split_active = false;
            return;
        }
        if self.scanline_detection_pending {
            self.scanline_detection_pending = false;
            if self.in_frame {
                self.irq_scanline = self.irq_scanline.wrapping_add(1);
            } else {
                self.in_frame = true;
                self.irq_scanline = 0;
            }
            if self.irq_compare != 0 && self.irq_scanline == self.irq_compare {
                self.irq_pending = true;
            }
            self.split_tile = 0;
        }
        if fetch.kind == PpuFetchKind::BackgroundNametable && fetch.rendering {
            let tile = self.split_tile;
            self.split_tile = self.split_tile.saturating_add(1);
            let threshold = self.split_control & 0x1f;
            let on_right = self.split_control & 0x40 != 0;
            self.split_active = self.split_control & 0x80 != 0
                && self.exram_mode < 2
                && self.pin_92
                && if on_right {
                    tile >= threshold
                } else {
                    tile < threshold
                };
            let row = (usize::from(self.split_scroll) + usize::from(self.irq_scanline)) % 240 / 8;
            let column = usize::from(tile) & 31;
            self.split_exram_offset = row * 32 + column;
            self.extended_attribute = self.exram[address as usize & 0x03ff];
        }
        if (0x2000..0x3000).contains(&address) {
            let normalized = address & 0x2fff;
            if normalized == self.last_nt_address {
                self.matching_nt_reads = self.matching_nt_reads.saturating_add(1);
            } else {
                self.last_nt_address = normalized;
                self.matching_nt_reads = 1;
            }
            if self.matching_nt_reads == 3 {
                self.scanline_detection_pending = true;
            }
        } else {
            self.matching_nt_reads = 0;
        }
    }
    fn notify_ppu_address(&mut self, address: u16, cycle: u64, is_read: bool) {
        self.notify_ppu_fetch(PpuFetch::address_only(address, cycle, is_read));
    }
    fn clock_cpu(&mut self) {
        if self.saw_ppu_this_cpu_cycle {
            self.idle_cpu_cycles = 0;
        } else {
            self.idle_cpu_cycles = self.idle_cpu_cycles.saturating_add(1);
        }
        self.saw_ppu_this_cpu_cycle = false;
        if !self.pin_92 {
            self.in_frame = false;
            self.irq_scanline = 0;
            self.matching_nt_reads = 0;
            self.scanline_detection_pending = false;
            self.split_active = false;
        }
        if self.idle_cpu_cycles >= 3 {
            self.in_frame = false;
            self.matching_nt_reads = 0;
            self.scanline_detection_pending = false;
            self.split_active = false;
        }

        self.pin_read_strobe = false;
        self.pin_write_strobe = false;
        if self.timer_running {
            self.timer_counter -= 1;
            if self.timer_counter == 0 {
                self.timer_running = false;
                self.timer_irq_pending = true;
            }
        }
        self.pulse_clock_half = !self.pulse_clock_half;
        if self.pulse_clock_half {
            self.pulses[0].clock_timer();
            self.pulses[1].clock_timer();
        }
        self.audio_frame_divider += 1;
        if self.audio_frame_divider >= 7424 {
            self.audio_frame_divider = 0;
            self.pulses[0].clock_frame();
            self.pulses[1].clock_frame();
        }
    }
    fn irq_pending(&self) -> bool {
        self.irq_enabled && self.irq_pending
            || self.pcm_irq_enabled && self.pcm_irq_pending
            || self.timer_irq_pending
    }
    fn expansion_audio_sample(&self) -> f32 {
        let pulse = f32::from(self.pulses[0].sample() + self.pulses[1].sample()) / 30.0;
        let pcm = f32::from(self.pcm_value) / 255.0;
        pulse + pcm
    }
    fn external_pin_read(&self, pin: u8) -> bool {
        match pin {
            92 => self.pin_92,
            97 => self.pin_level(0),
            98 => self.pin_level(1),
            _ => true,
        }
    }
    fn external_pin_write(&mut self, pin: u8, level: bool) {
        match pin {
            92 => {
                self.pin_92 = level;
                if !level {
                    // Pin 92 disables substitutions and holds the scanline
                    // detector and its counter in reset. An already-pending
                    // IRQ is not acknowledged by the pin.
                    self.in_frame = false;
                    self.irq_scanline = 0;
                    self.matching_nt_reads = 0;
                    self.scanline_detection_pending = false;
                    self.split_active = false;
                }
            }
            97 => self.pin_inputs[0] = level,
            98 => self.pin_inputs[1] = level,
            _ => {}
        }
    }
    fn reset(&mut self, kind: ResetKind) {
        if kind == ResetKind::PowerOn {
            let config = self.config.clone();
            *self = Self::new(config).unwrap_or_else(|_| {
                unreachable!("an already-validated MMC5 configuration must remain valid")
            });
            return;
        }

        // The M2-gap reset detector preserves all MMC5 state except the
        // scanline IRQ enable. MMC5A's independent timer is also stopped and
        // acknowledged by the same reset condition.
        self.irq_enabled = false;
        self.timer_counter = 0;
        self.timer_running = false;
        self.timer_irq_pending = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Mirroring, TimingMode};

    fn config() -> CartridgeConfig {
        CartridgeConfig {
            mapper_id: 5,
            submapper: 0,
            prg_rom_len: 0x100000,
            chr_rom_len: 0x100000,
            prg_ram_len: 0x10000,
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

    fn fetch(kind: PpuFetchKind, address: u16, rendering: bool, sprite_16: bool) -> PpuFetch {
        PpuFetch {
            address,
            ppu_cycle: 0,
            kind,
            rendering,
            sprite_16,
            is_read: true,
        }
    }

    #[test]
    fn mode_three_maps_rom_and_ram_and_honors_write_key() {
        let mut mapper = M005::new(config()).unwrap();
        mapper.cpu_write(0x5114, 0x82, None);
        mapper.cpu_write(0x5115, 0x03, None);
        assert_eq!(mapper.cpu_peek(0x8000), ReadMapping::PrgRom(0x4000));
        assert!(matches!(mapper.cpu_peek(0xa000), ReadMapping::PrgRam(_)));
        assert_eq!(mapper.cpu_write(0xa000, 1, None), WriteMapping::Handled);
        mapper.cpu_write(0x5102, 2, None);
        mapper.cpu_write(0x5103, 1, None);
        assert!(matches!(
            mapper.cpu_write(0xa000, 1, None),
            WriteMapping::PrgRam(_)
        ));
    }

    #[test]
    fn supports_arbitrary_nametables_fill_and_multiplier() {
        let mut mapper = M005::new(config()).unwrap();
        mapper.cpu_write(0x5105, 0xe4, None);
        mapper.cpu_write(0x5106, 0x2a, None);
        mapper.cpu_write(0x5107, 2, None);
        assert_eq!(
            mapper.nametable_target(0x2800),
            NametableTarget::MapperRam(0)
        );
        assert_eq!(
            mapper.nametable_target(0x2c00),
            NametableTarget::Fill {
                tile: 0x2a,
                attribute: 0xaa
            }
        );
        mapper.cpu_write(0x5205, 13, None);
        mapper.cpu_write(0x5206, 20, None);
        assert_eq!(
            mapper.cpu_peek(0x5205),
            ReadMapping::Value(BusValue::full(4))
        );
        assert_eq!(
            mapper.cpu_peek(0x5206),
            ReadMapping::Value(BusValue::full(1))
        );
    }

    #[test]
    fn scanline_irq_uses_three_reads_then_the_following_fetch() {
        let mut mapper = M005::new(config()).unwrap();
        mapper.cpu_write(0x5203, 1, None);

        for cycle in 0..3 {
            mapper.notify_ppu_address(0x2123, cycle, true);
        }
        assert!(!mapper.in_frame);
        mapper.notify_ppu_address(0x23c0, 3, true);
        assert!(mapper.in_frame);
        assert_eq!(mapper.irq_scanline, 0);

        for cycle in 4..7 {
            mapper.notify_ppu_address(0x2456, cycle, true);
        }
        mapper.notify_ppu_address(0x27c0, 7, true);
        assert!(mapper.irq_pending);
        assert!(!mapper.irq_pending());

        mapper.cpu_write(0x5204, 0x80, None);
        assert!(mapper.irq_pending());
        assert_eq!(
            mapper.cpu_read(0x5204),
            ReadMapping::Value(BusValue::full(0xc0))
        );
        assert!(!mapper.irq_pending());
    }

    #[test]
    fn ppu_writes_do_not_clock_read_based_scanline_detection() {
        let mut mapper = M005::new(config()).unwrap();
        for cycle in 0..4 {
            mapper.notify_ppu_address(0x2123, cycle, false);
        }
        assert!(!mapper.in_frame);
        assert_eq!(mapper.matching_nt_reads, 0);
    }

    #[test]
    fn every_prg_mode_and_physical_ram_wiring_maps_exactly() {
        let mut mapper = M005::new(config()).unwrap();
        for (mode, registers, expected) in [
            (0, [0, 0, 0, 0x8c], [12, 13, 14, 15]),
            (1, [0, 0x86, 0, 0x8a], [6, 7, 10, 11]),
            (2, [0, 0x86, 0x89, 0x8a], [6, 7, 9, 10]),
            (3, [0x84, 0x85, 0x86, 0x87], [4, 5, 6, 7]),
        ] {
            mapper.cpu_write(0x5100, mode, None);
            for (slot, value) in registers.into_iter().enumerate() {
                mapper.cpu_write(0x5114 + slot as u16, value, None);
            }
            for (slot, bank) in expected.into_iter().enumerate() {
                assert_eq!(
                    mapper.cpu_peek(0x8000 + slot as u16 * 0x2000),
                    ReadMapping::PrgRom(bank * 0x2000)
                );
            }
        }

        let mut eight = config();
        eight.prg_ram_len = 0x2000;
        let mut eight = M005::new(eight).unwrap();
        eight.cpu_write(0x5113, 3, None);
        assert_eq!(eight.cpu_peek(0x6000), ReadMapping::PrgRam(0));
        eight.cpu_write(0x5113, 4, None);
        assert_eq!(eight.cpu_peek(0x6000), ReadMapping::OpenBus);

        let mut mixed = config();
        mixed.prg_ram_len = 0x2000;
        mixed.prg_nvram_len = 0x2000;
        let mut mixed = M005::new(mixed).unwrap();
        mixed.cpu_write(0x5113, 0, None);
        assert_eq!(mixed.cpu_peek(0x6000), ReadMapping::PrgRam(0x2000));
        mixed.cpu_write(0x5113, 4, None);
        assert_eq!(mixed.cpu_peek(0x6000), ReadMapping::PrgRam(0));

        let mut thirty_two = config();
        thirty_two.prg_ram_len = 0x8000;
        let mut thirty_two = M005::new(thirty_two).unwrap();
        thirty_two.cpu_write(0x5113, 3, None);
        assert_eq!(thirty_two.cpu_peek(0x6000), ReadMapping::PrgRam(0x6000));
        thirty_two.cpu_write(0x5113, 7, None);
        assert_eq!(thirty_two.cpu_peek(0x6000), ReadMapping::OpenBus);
    }

    #[test]
    fn chr_capture_fetch_sets_and_extended_attributes_are_independent() {
        let mut mapper = M005::new(config()).unwrap();
        mapper.cpu_write(0x5101, 0, None);
        mapper.cpu_write(0x5127, 0x41, None);
        assert_eq!(mapper.chr_banks[7], 0x209);
        mapper.cpu_write(0x5101, 1, None);
        mapper.cpu_write(0x5127, 0x81, None);
        assert_eq!(mapper.chr_banks[7], 0x205);
        mapper.cpu_write(0x5130, 1, None);
        mapper.cpu_write(0x5101, 2, None);
        mapper.cpu_write(0x5127, 0x81, None);
        assert_eq!(mapper.chr_banks[7], 0x303);
        mapper.cpu_write(0x5130, 2, None);
        mapper.cpu_write(0x5101, 3, None);
        mapper.cpu_write(0x5127, 0x81, None);
        assert_eq!(mapper.chr_banks[7], 0x281);

        mapper.cpu_write(0x5130, 0, None);
        mapper.cpu_write(0x5120, 1, None);
        mapper.cpu_write(0x5128, 2, None);
        mapper.notify_ppu_fetch(fetch(PpuFetchKind::SpritePattern, 0, true, true));
        assert_eq!(mapper.ppu_peek(0), ReadMapping::ChrRom(0x400));
        mapper.notify_ppu_fetch(fetch(PpuFetchKind::BackgroundPattern, 0, true, true));
        assert_eq!(mapper.ppu_peek(0), ReadMapping::ChrRom(0x800));
        mapper.notify_ppu_fetch(fetch(PpuFetchKind::Cpu, 0, false, false));
        assert_eq!(mapper.ppu_peek(0), ReadMapping::ChrRom(0x800));

        mapper.cpu_write(0x5104, 2, None);
        mapper.cpu_write(0x5c00, 0x83, None);
        mapper.cpu_write(0x5104, 1, None);
        mapper.cpu_write(0x5130, 2, None);
        mapper.notify_ppu_fetch(fetch(
            PpuFetchKind::BackgroundNametable,
            0x2000,
            true,
            false,
        ));
        mapper.notify_ppu_fetch(fetch(
            PpuFetchKind::BackgroundAttribute,
            0x23c0,
            true,
            false,
        ));
        assert_eq!(
            mapper.nametable_target(0x23c0),
            NametableTarget::Fill {
                tile: 0,
                attribute: 0xaa
            }
        );
        mapper.notify_ppu_fetch(fetch(PpuFetchKind::BackgroundPattern, 0, true, false));
        assert_eq!(mapper.ppu_peek(0), ReadMapping::ChrRom(0x83000));
    }

    #[test]
    fn exram_access_modes_follow_rendering_state() {
        let mut mapper = M005::new(config()).unwrap();
        mapper.cpu_write(0x5104, 0, None);
        mapper.cpu_write(0x5c00, 0x11, None);
        assert_eq!(mapper.exram[0], 0);
        assert_eq!(mapper.cpu_peek(0x5c00), ReadMapping::OpenBus);

        mapper.notify_ppu_fetch(fetch(
            PpuFetchKind::BackgroundNametable,
            0x2000,
            true,
            false,
        ));
        mapper.cpu_write(0x5c00, 0x22, None);
        assert_eq!(mapper.exram[0], 0x22);
        mapper.cpu_write(0x5104, 2, None);
        assert_eq!(
            mapper.cpu_peek(0x5c00),
            ReadMapping::Value(BusValue::full(0x22))
        );
        mapper.cpu_write(0x5c00, 0x33, None);
        mapper.cpu_write(0x5104, 3, None);
        mapper.cpu_write(0x5c00, 0x44, None);
        assert_eq!(mapper.exram[0], 0x33);
        assert_eq!(
            mapper.cpu_peek(0x5c00),
            ReadMapping::Value(BusValue::full(0x33))
        );
    }

    #[test]
    fn pcm_pulses_timer_and_external_strobes_share_the_irq_and_audio_paths() {
        let mut mapper = M005::new(config()).unwrap();
        mapper.cpu_write(0x5015, 1, None);
        mapper.cpu_write(0x5000, 0xdf, None);
        mapper.cpu_write(0x5002, 8, None);
        mapper.cpu_write(0x5003, 0, None);
        assert!(mapper.expansion_audio_sample() > 0.0);

        mapper.cpu_write(0x5010, 0x81, None);
        mapper.cpu_read_result(0x8000, 0);
        assert!(mapper.irq_pending());
        assert_eq!(
            mapper.cpu_read(0x5010),
            ReadMapping::Value(BusValue::full(0x81))
        );
        assert!(!mapper.irq_pending());
        mapper.cpu_read_result(0x8000, 0x80);
        assert_eq!(mapper.pcm_value, 0x80);

        mapper.cpu_write(0x520a, 0, None);
        mapper.cpu_write(0x5209, 2, None);
        mapper.clock_cpu();
        assert!(!mapper.timer_irq_pending);
        mapper.clock_cpu();
        assert!(mapper.irq_pending());
        assert_eq!(
            mapper.cpu_read(0x5209),
            ReadMapping::Value(BusValue::full(0x80))
        );
        assert!(!mapper.timer_irq_pending);

        mapper.external_pin_write(97, false);
        assert!(!mapper.external_pin_read(97));
        mapper.cpu_write(0x5207, 3, None);
        assert_eq!(mapper.cpu_read(0x5800), ReadMapping::OpenBus);
        assert!(!mapper.external_pin_read(97));
        mapper.clock_cpu();
        assert!(mapper.external_pin_read(97));
        mapper.cpu_write(0x5800, 0, None);
        assert!(!mapper.external_pin_read(98));
        mapper.clock_cpu();
        assert!(mapper.external_pin_read(98));
        assert!(mapper.external_pin_read(99));
    }

    #[test]
    fn reset_button_preserves_registers_and_pending_scanline_irq() {
        let mut mapper = M005::new(config()).unwrap();
        mapper.cpu_write(0x5100, 0, None);
        mapper.cpu_write(0x5101, 1, None);
        mapper.cpu_write(0x5117, 0x8c, None);
        mapper.cpu_write(0x5203, 7, None);
        mapper.cpu_write(0x5204, 0x80, None);
        mapper.irq_pending = true;
        mapper.in_frame = true;
        mapper.irq_scanline = 5;
        mapper.cpu_write(0x5209, 1, None);
        mapper.clock_cpu();
        assert!(mapper.timer_irq_pending);
        assert!(mapper.irq_pending());

        mapper.reset(ResetKind::ResetButton);

        assert_eq!(mapper.prg_mode, 0);
        assert_eq!(mapper.chr_mode, 1);
        assert_eq!(mapper.prg_banks[4], 0x8c);
        assert_eq!(mapper.irq_compare, 7);
        assert_eq!(mapper.irq_scanline, 5);
        assert!(mapper.irq_pending);
        assert!(mapper.in_frame);
        assert!(!mapper.irq_enabled);
        assert!(!mapper.timer_running);
        assert!(!mapper.timer_irq_pending);
        assert!(!mapper.irq_pending());
    }

    #[test]
    fn power_on_defaults_and_pin_92_match_mmc5a_measurements() {
        let mut mapper = M005::new(config()).unwrap();
        assert_eq!(
            mapper.cpu_peek(0x5205),
            ReadMapping::Value(BusValue::full(1))
        );
        assert_eq!(
            mapper.cpu_peek(0x5206),
            ReadMapping::Value(BusValue::full(0xfe))
        );

        mapper.cpu_write(0x5120, 1, None);
        mapper.cpu_write(0x5128, 2, None);
        mapper.notify_ppu_fetch(fetch(PpuFetchKind::BackgroundPattern, 0, true, true));
        assert_eq!(mapper.ppu_peek(0), ReadMapping::ChrRom(0x800));

        mapper.external_pin_write(92, false);
        mapper.notify_ppu_fetch(fetch(PpuFetchKind::BackgroundPattern, 0, true, true));
        assert_eq!(mapper.ppu_peek(0), ReadMapping::ChrRom(0x400));
        for cycle in 0..4 {
            mapper.notify_ppu_address(0x2123, cycle, true);
        }
        assert!(!mapper.in_frame);
        assert_eq!(mapper.irq_scanline, 0);

        mapper.cpu_write(0x5205, 0, None);
        mapper.cpu_write(0x5206, 0, None);
        mapper.reset(ResetKind::PowerOn);
        assert_eq!(mapper.prg_mode, 3);
        assert_eq!(mapper.prg_banks[4], 0xff);
        assert_eq!(
            mapper.cpu_peek(0x5205),
            ReadMapping::Value(BusValue::full(1))
        );
        assert_eq!(
            mapper.cpu_peek(0x5206),
            ReadMapping::Value(BusValue::full(0xfe))
        );
        assert!(mapper.external_pin_read(92));
    }

    #[test]
    fn rejects_simultaneous_chr_rom_and_ram_devices() {
        let mut mixed = config();
        mixed.chr_ram_len = 0x2000;
        assert!(matches!(
            M005::new(mixed),
            Err(MapperError::InvalidConfiguration { mapper_id: 5, .. })
        ));
    }
}
