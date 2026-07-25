pub mod cartridge_access;
pub mod error;

use crate::{
    BusValue, Mapper, NametableTarget, PpuFetch, PpuFetchKind, ReadMapping, ResetKind,
    WriteMapping, create_mapper,
};

use cartridge_access::CartridgeAccess;
use error::CartridgeParseError;

pub use crate::{InesHeader as Header, TvSystem};

pub type Result<T> = std::result::Result<T, CartridgeParseError>;

fn try_get_next_n<'a>(data: &mut &'a [u8], n: usize) -> Result<&'a [u8]> {
    if data.len() < n {
        return Err(CartridgeParseError::NotEnoughBytesError(n));
    }
    let (head, tail) = data.split_at(n);
    *data = tail;
    Ok(head)
}

pub struct Cartridge {
    mapper: Box<dyn Mapper>,
    header: Header,
    prg_rom: Vec<u8>,
    chr_rom: Vec<u8>,
    prg_ram: Vec<u8>,
    chr_ram: Vec<u8>,
}

impl Cartridge {
    pub fn get_header(&self) -> &Header {
        &self.header
    }

    pub fn from_file(filename: &str) -> Result<Self> {
        let bytes = std::fs::read(filename)?;
        Self::from_bytes(&bytes)
    }

    pub fn from_bytes(mut bytes: &[u8]) -> Result<Self> {
        let header = Header::parse(try_get_next_n(&mut bytes, 16)?)?;
        let config = header.config().clone();

        let trainer = if config.has_trainer {
            Some(try_get_next_n(&mut bytes, 512)?)
        } else {
            None
        };

        let prg_rom = try_get_next_n(&mut bytes, config.prg_rom_len)?.to_vec();
        let chr_rom = try_get_next_n(&mut bytes, config.chr_rom_len)?.to_vec();
        let mut prg_ram = vec![0; config.total_prg_ram_len()];
        let chr_ram = vec![0; config.total_chr_ram_len()];

        let mapper = create_mapper(config)?;

        if let Some(trainer) = trainer {
            // A trainer is presented at CPU $7000-$71FF.  It only initializes
            // storage when the selected board actually decodes that range;
            // trainer metadata must never manufacture a RAM chip.
            for (index, value) in trainer.iter().copied().enumerate() {
                if let ReadMapping::PrgRam(offset) = mapper.cpu_peek(0x7000 + index as u16)
                    && let Some(target) = prg_ram.get_mut(offset)
                {
                    *target = value;
                }
            }
        }
        Ok(Self {
            mapper,
            header,
            prg_rom,
            chr_rom,
            prg_ram,
            chr_ram,
        })
    }

    pub fn write(&mut self, access: CartridgeAccess, value: u8) {
        let mapping = match access {
            CartridgeAccess::CpuAccess { address } => {
                let rom_bus_value = self.read_mapping(self.mapper.cpu_peek(address), 0).ok();
                self.mapper.cpu_write(address, value, rom_bus_value)
            }
            CartridgeAccess::PpuAccess { address } => self.mapper.ppu_write(address, value),
        };
        self.write_mapping(mapping, value);
    }

    pub fn read(&mut self, access: CartridgeAccess) -> Option<u8> {
        self.read_with_open_bus(access, 0)
    }

    pub fn read_with_open_bus(&mut self, access: CartridgeAccess, open_bus: u8) -> Option<u8> {
        let (mapping, cpu_address) = match access {
            CartridgeAccess::CpuAccess { address } => {
                (self.mapper.cpu_read(address), Some(address))
            }
            CartridgeAccess::PpuAccess { address } => (self.mapper.ppu_read(address), None),
        };
        let value = self.read_mapping(mapping, open_bus).ok();
        if let (Some(address), Some(value)) = (cpu_address, value) {
            self.mapper.cpu_read_result(address, value);
        }
        value
    }

    pub fn peek_with_open_bus(&self, access: CartridgeAccess, open_bus: u8) -> Option<u8> {
        let mapping = match access {
            CartridgeAccess::CpuAccess { address } => self.mapper.cpu_peek(address),
            CartridgeAccess::PpuAccess { address } => self.mapper.ppu_peek(address),
        };
        self.read_mapping(mapping, open_bus).ok()
    }

    fn read_mapping(&self, mapping: ReadMapping, open_bus: u8) -> std::result::Result<u8, ()> {
        match mapping {
            ReadMapping::PrgRom(offset) => self.prg_rom.get(offset).copied().ok_or(()),
            ReadMapping::PrgRam(offset) => self.prg_ram.get(offset).copied().ok_or(()),
            ReadMapping::ChrRom(offset) => self.chr_rom.get(offset).copied().ok_or(()),
            ReadMapping::ChrRam(offset) => self.chr_ram.get(offset).copied().ok_or(()),
            ReadMapping::Value(BusValue { value, driven_mask }) => {
                Ok((value & driven_mask) | (open_bus & !driven_mask))
            }
            ReadMapping::OpenBus => Err(()),
        }
    }

    fn write_mapping(&mut self, mapping: WriteMapping, value: u8) {
        match mapping {
            WriteMapping::PrgRam(offset) => {
                if let Some(target) = self.prg_ram.get_mut(offset) {
                    *target = value;
                }
            }
            WriteMapping::ChrRam(offset) => {
                if let Some(target) = self.chr_ram.get_mut(offset) {
                    *target = value;
                }
            }
            WriteMapping::Handled | WriteMapping::Unmapped => {}
        }
    }

    pub fn map_nametable(&self, address: u16) -> u16 {
        match self.mapper.nametable_target(address) {
            NametableTarget::Ciram(offset) | NametableTarget::CartridgeVram(offset) => {
                0x2000 | (offset as u16 & 0x0fff)
            }
            NametableTarget::ChrRom(_) | NametableTarget::ChrRam(_) => 0x2000,
            NametableTarget::MapperRam(offset) => 0x2000 | (offset as u16 & 0x0fff),
            NametableTarget::Fill { .. } | NametableTarget::OpenBus => 0x2000,
        }
    }

    pub fn read_mapper_nametable(&self, address: u16) -> Option<u8> {
        match self.mapper.nametable_target(address) {
            NametableTarget::MapperRam(offset) => {
                Some(self.mapper.mapper_ram_read(offset).resolve(0))
            }
            NametableTarget::ChrRom(offset) => self.chr_rom.get(offset).copied(),
            NametableTarget::ChrRam(offset) => self.chr_ram.get(offset).copied(),
            NametableTarget::Fill { tile, attribute } => {
                let offset = address.wrapping_sub(0x2000) & 0x03ff;
                Some(if offset >= 0x03c0 { attribute } else { tile })
            }
            NametableTarget::OpenBus => Some(0),
            NametableTarget::Ciram(_) | NametableTarget::CartridgeVram(_) => None,
        }
    }

    pub fn write_mapper_nametable(&mut self, address: u16, value: u8) -> bool {
        match self.mapper.nametable_target(address) {
            NametableTarget::MapperRam(offset) => {
                self.mapper.mapper_ram_write(offset, value);
                true
            }
            NametableTarget::ChrRom(_)
            | NametableTarget::Fill { .. }
            | NametableTarget::OpenBus => true,
            NametableTarget::ChrRam(offset) => {
                if let Some(target) = self.chr_ram.get_mut(offset) {
                    *target = value;
                }
                true
            }
            NametableTarget::Ciram(_) | NametableTarget::CartridgeVram(_) => false,
        }
    }

    pub fn ppu_ciram_target(&self, address: u16) -> Option<usize> {
        self.mapper.ppu_ciram_target(address)
    }

    pub fn clock_cpu(&mut self) {
        self.mapper.clock_cpu();
    }

    pub fn notify_ppu_address(&mut self, address: u16, ppu_cycle: u64, is_read: bool) {
        self.mapper.notify_ppu_address(address, ppu_cycle, is_read);
    }

    pub fn notify_ppu_fetch(&mut self, fetch: PpuFetch) {
        self.mapper.notify_ppu_fetch(fetch);
    }

    pub fn notify_ppu_read(
        &mut self,
        address: u16,
        ppu_cycle: u64,
        kind: PpuFetchKind,
        rendering: bool,
    ) {
        self.notify_ppu_fetch(PpuFetch {
            address,
            ppu_cycle,
            kind,
            rendering,
            sprite_16: false,
            is_read: true,
        });
    }

    pub fn irq_pending(&self) -> bool {
        self.mapper.irq_pending()
    }

    pub fn expansion_audio_sample(&self) -> f32 {
        self.mapper.expansion_audio_sample()
    }

    /// Controls a physical cartridge PRG-RAM isolation switch, such as the
    /// Family BASIC backup switch. This does not perform a CPU bus write.
    pub fn set_prg_ram_enabled(&mut self, enabled: bool) {
        self.mapper.set_prg_ram_enabled(enabled);
    }

    pub fn external_pin_read(&self, pin: u8) -> bool {
        self.mapper.external_pin_read(pin)
    }

    pub fn external_pin_write(&mut self, pin: u8, level: bool) {
        self.mapper.external_pin_write(pin, level);
    }

    pub fn reset_mapper(&mut self, kind: ResetKind) {
        self.mapper.reset(kind);
    }

    /// Export every nonvolatile memory region in deterministic hardware order:
    /// PRG NVRAM, CHR NVRAM, then mapper-chip-internal persistent RAM.
    pub fn save_ram(&self) -> Vec<u8> {
        let config = self.mapper.config();
        let prg_start = config.prg_ram_len.min(self.prg_ram.len());
        let prg_end = prg_start
            .saturating_add(config.prg_nvram_len)
            .min(self.prg_ram.len());
        let chr_start = config.chr_ram_len.min(self.chr_ram.len());
        let chr_end = chr_start
            .saturating_add(config.chr_nvram_len)
            .min(self.chr_ram.len());
        let mut save = Vec::with_capacity(
            (prg_end - prg_start)
                + (chr_end - chr_start)
                + self.mapper.persistent_mapper_ram().len(),
        );
        save.extend_from_slice(&self.prg_ram[prg_start..prg_end]);
        save.extend_from_slice(&self.chr_ram[chr_start..chr_end]);
        save.extend_from_slice(self.mapper.persistent_mapper_ram());
        save
    }

    pub fn load_save_ram(&mut self, bytes: &[u8]) {
        let config = self.mapper.config();
        let prg_start = config.prg_ram_len.min(self.prg_ram.len());
        let prg_len = config
            .prg_nvram_len
            .min(self.prg_ram.len().saturating_sub(prg_start));
        let chr_start = config.chr_ram_len.min(self.chr_ram.len());
        let chr_len = config
            .chr_nvram_len
            .min(self.chr_ram.len().saturating_sub(chr_start));
        let prg_copy_len = prg_len.min(bytes.len());
        self.prg_ram[prg_start..prg_start + prg_copy_len].copy_from_slice(&bytes[..prg_copy_len]);
        let chr_source_start = prg_len.min(bytes.len());
        let chr_copy_len = chr_len.min(bytes.len().saturating_sub(chr_source_start));
        self.chr_ram[chr_start..chr_start + chr_copy_len]
            .copy_from_slice(&bytes[chr_source_start..chr_source_start + chr_copy_len]);
        let mapper_source_start = prg_len.saturating_add(chr_len).min(bytes.len());
        self.mapper
            .load_persistent_mapper_ram(&bytes[mapper_source_start..]);
    }
}
