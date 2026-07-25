//! NES cartridge mapper implementations.
//!
//! This crate deliberately has no dependency on the emulator. It translates
//! CPU and PPU bus activity into cartridge memory targets and exposes mapper
//! side effects such as mirroring, IRQs, and expansion audio.

pub mod cartridge;
mod config;
mod mapper;
pub mod mappers;
mod mapping;

pub use config::{CartridgeConfig, HeaderError, InesHeader, Mirroring, TimingMode, TvSystem};
pub use mapper::{Mapper, MapperError, PpuFetch, PpuFetchKind, ResetKind, create_mapper};
pub use mappers::M018Resources;
pub use mapping::{BusValue, NametableTarget, ReadMapping, WriteMapping};
