use thiserror::Error;

use crate::{
    BusValue, CartridgeConfig, NametableTarget, ReadMapping, WriteMapping,
    mappers::{
        M000, M001, M002, M003, M004, M005, M007, M009, M011, M016, M018, M019, M023, M066, M069,
        M071, M079, M087, M206, M210,
    },
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResetKind {
    PowerOn,
    ResetButton,
}

/// The bus operation that caused a PPU cartridge access.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PpuFetchKind {
    /// A CPU access through PPUDATA rather than a renderer fetch.
    Cpu,
    BackgroundNametable,
    BackgroundAttribute,
    BackgroundPattern,
    SpritePattern,
    /// Renderer fetch whose value is discarded (including the two end-of-line
    /// nametable fetches).
    Dummy,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PpuFetch {
    pub address: u16,
    pub ppu_cycle: u64,
    pub kind: PpuFetchKind,
    pub rendering: bool,
    pub sprite_16: bool,
    pub is_read: bool,
}

impl PpuFetch {
    pub const fn address_only(address: u16, ppu_cycle: u64, is_read: bool) -> Self {
        Self {
            address,
            ppu_cycle,
            kind: PpuFetchKind::Other,
            rendering: false,
            sprite_16: false,
            is_read,
        }
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum MapperError {
    #[error("unsupported mapper {mapper_id}, submapper {submapper}")]
    Unsupported { mapper_id: u16, submapper: u8 },
    #[error("invalid cartridge configuration for mapper {mapper_id}: {reason}")]
    InvalidConfiguration {
        mapper_id: u16,
        reason: &'static str,
    },
}

pub trait Mapper: std::fmt::Debug {
    fn config(&self) -> &CartridgeConfig;
    fn cpu_read(&mut self, address: u16) -> ReadMapping;
    /// Supplies the resolved bus byte after a non-peek CPU read. Mappers with
    /// read-triggered hardware (MMC5 PCM) use this without coupling mapper
    /// address translation to cartridge storage ownership.
    fn cpu_read_result(&mut self, _address: u16, _value: u8) {}
    fn cpu_peek(&self, address: u16) -> ReadMapping;
    fn cpu_write(&mut self, address: u16, value: u8, rom_bus_value: Option<u8>) -> WriteMapping;
    fn ppu_read(&mut self, address: u16) -> ReadMapping;
    fn ppu_peek(&self, address: u16) -> ReadMapping;
    fn ppu_write(&mut self, address: u16, value: u8) -> WriteMapping;

    fn nametable_target(&self, address: u16) -> NametableTarget {
        let offset = self.config().mirroring.map_ciram(address);
        if self.config().mirroring == crate::Mirroring::FourScreen {
            NametableTarget::CartridgeVram(offset)
        } else {
            NametableTarget::Ciram(offset)
        }
    }

    fn mapper_ram_read(&self, _offset: usize) -> BusValue {
        BusValue::full(0)
    }
    fn mapper_ram_write(&mut self, _offset: usize, _value: u8) {}

    /// Mapper-chip-internal nonvolatile memory that is not represented by the
    /// NES 2.0 PRG/CHR NVRAM size fields.
    fn persistent_mapper_ram(&self) -> &[u8] {
        &[]
    }
    fn load_persistent_mapper_ram(&mut self, _bytes: &[u8]) {}

    /// Some boards, notably Namco 163, can expose the console's CIRAM in the
    /// pattern-table address range. The PPU owns CIRAM, so this target cannot
    /// be represented as cartridge memory.
    fn ppu_ciram_target(&self, _address: u16) -> Option<usize> {
        None
    }

    fn clock_cpu(&mut self) {}
    /// Rich PPU bus notification.  The default forwards to the legacy
    /// address-only callback so out-of-tree mappers can migrate gradually.
    fn notify_ppu_fetch(&mut self, fetch: PpuFetch) {
        self.notify_ppu_address(fetch.address, fetch.ppu_cycle, fetch.is_read);
    }
    fn notify_ppu_address(&mut self, _address: u16, _ppu_cycle: u64, _is_read: bool) {}
    fn irq_pending(&self) -> bool {
        false
    }
    fn expansion_audio_sample(&self) -> f32 {
        0.0
    }
    /// Set the state of a board-level PRG-RAM isolation switch, when present.
    /// This is an external hardware input, not a mapper register.
    fn set_prg_ram_enabled(&mut self, _enabled: bool) {}
    /// Board-level peripheral pins. Disconnected inputs deterministically read
    /// high, matching passive pull-ups used by MMC5A boards.
    fn external_pin_read(&self, _pin: u8) -> bool {
        true
    }
    fn external_pin_write(&mut self, _pin: u8, _level: bool) {}
    fn reset(&mut self, _kind: ResetKind) {}
}

pub fn create_mapper(config: CartridgeConfig) -> Result<Box<dyn Mapper>, MapperError> {
    Ok(match config.mapper_id {
        0 => Box::new(M000::new(config)?),
        1 => Box::new(M001::new(config)?),
        2 => Box::new(M002::new(config)?),
        3 => Box::new(M003::new(config)?),
        4 => Box::new(M004::new(config)?),
        5 => Box::new(M005::new(config)?),
        7 => Box::new(M007::new(config)?),
        9 => Box::new(M009::new(config)?),
        11 => Box::new(M011::new(config)?),
        16 => Box::new(M016::new(config)?),
        18 => Box::new(M018::new(config)?),
        19 => Box::new(M019::new(config)?),
        23 => Box::new(M023::new(config)?),
        66 => Box::new(M066::new(config)?),
        69 => Box::new(M069::new(config)?),
        71 => Box::new(M071::new(config)?),
        79 => Box::new(M079::new(config)?),
        87 => Box::new(M087::new(config)?),
        206 => Box::new(M206::new(config)?),
        210 => Box::new(M210::new(config)?),
        mapper_id => {
            return Err(MapperError::Unsupported {
                mapper_id,
                submapper: config.submapper,
            });
        }
    })
}
