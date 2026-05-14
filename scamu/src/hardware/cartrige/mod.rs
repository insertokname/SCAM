pub mod cartrige_access;
pub mod error;
mod mappers;

use crate::hardware::{
    cartrige::{cartrige_access::CartrigeAccess, error::CartrigeParseError, mappers::Mapper},
    constants::cartrige::*,
};

pub type Result<T> = std::result::Result<T, CartrigeParseError>;

fn try_get_next_n<'a>(data_ptr: &mut &'a [u8], n: usize) -> Result<&'a [u8]> {
    if data_ptr.len() < n {
        return Err(CartrigeParseError::NotEnoughBytesError(n));
    } else {
        let start = data_ptr.get(0..n);
        *data_ptr = &data_ptr[n..];
        start.ok_or_else(|| CartrigeParseError::NotEnoughBytesError(n))
    }
}

fn try_get_next(data_ptr: &mut &[u8]) -> Result<u8> {
    if data_ptr.len() < 1 {
        return Err(CartrigeParseError::NotEnoughBytesError(1));
    } else {
        let start = data_ptr.get(0);
        *data_ptr = &data_ptr[1..];
        start
            .cloned()
            .ok_or_else(|| CartrigeParseError::NotEnoughBytesError(1))
    }
}

pub struct Cartrige {
    mapper: Box<dyn Mapper>,
    header: Header,
    prg_mem: Vec<u8>,
    chr_mem: Vec<u8>,
}

impl Cartrige {
    pub fn get_header(&self) -> &Header {
        &self.header
    }

    pub fn from_file(filename: &str) -> Result<Self> {
        let bytes = std::fs::read(filename)?;
        Cartrige::from_bytes(bytes.as_slice())
    }

    pub fn from_bytes(mut bytes: &[u8]) -> Result<Self> {
        let bytes_ptr: &mut &[u8] = &mut bytes;

        if try_get_next_n(bytes_ptr, 4)? != &NES_MAGIC_NUMBERS {
            return Err(CartrigeParseError::MissingMagicNumbersError);
        }

        let prg_size = try_get_next(bytes_ptr)?;
        let chr_size = try_get_next(bytes_ptr)?;
        let flags6 = try_get_next(bytes_ptr)?;
        let flags7 = try_get_next(bytes_ptr)?;
        let flags8 = try_get_next(bytes_ptr)?;
        let flags9 = try_get_next(bytes_ptr)?;
        let flags10 = try_get_next(bytes_ptr)?;
        let _ = try_get_next_n(bytes_ptr, 5)?;

        let header = Header {
            prg_size,
            chr_size,
            flags6,
            flags7,
            flags8,
            flags9,
            flags10,
        };

        if header.get_has_trainer() {
            let _ = try_get_next_n(bytes_ptr, 512)?;
        }

        let prg_mem = try_get_next_n(bytes_ptr, 16384 * prg_size as usize)?.to_vec();
        let chr_mem = try_get_next_n(bytes_ptr, 8192 * chr_size as usize)?.to_vec();

        let mapper = mappers::from_header(header.clone())?;

        Ok(Self {
            mapper,
            header,
            prg_mem,
            chr_mem,
        })
    }

    // TODO: impl writing to chr or prg mem
    pub fn write(&mut self, cartrige_access: CartrigeAccess, value: u8) {
        let _ = self.mapper.map_write(cartrige_access, value);
    }

    pub fn read(&mut self, cartrige_access: CartrigeAccess) -> Option<u8> {
        let addr = self.mapper.map_read(cartrige_access.clone())?;
        match cartrige_access {
            CartrigeAccess::CpuAccess { .. } => Some(self.prg_mem[addr as usize]),
            CartrigeAccess::PpuAccess { .. } => Some(self.chr_mem[addr as usize]),
        }
    }

    pub fn map_nametable(&self, address: u16) -> u16 {
        self.mapper.map_nametable(address)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TvSystem {
    Ntsc,
    Pal,
    DualCompatible,
    Unknown(u8),
}

#[derive(Clone)]
pub struct Header {
    prg_size: u8,
    chr_size: u8,
    flags6: u8,
    flags7: u8,
    flags8: u8,
    flags9: u8,
    flags10: u8,
}

impl Header {
    pub fn prg_rom_size(&self) -> u8 {
        self.prg_size
    }

    pub fn prg_chr_size(&self) -> u8 {
        self.chr_size
    }

    pub fn prg_rom_size_bytes(&self) -> usize {
        self.prg_size as usize * PRG_ROM_BANK_SIZE
    }

    pub fn chr_rom_size_bytes(&self) -> usize {
        self.chr_size as usize * CHR_ROM_BANK_SIZE
    }

    pub fn prg_ram_size_bytes(&self) -> usize {
        let units = if self.flags8 == 0 {
            1
        } else {
            self.flags8 as usize
        };
        units * PRG_RAM_BANK_SIZE
    }

    pub fn get_nametable_arrangement(&self) -> u8 {
        self.flags6 & FLAG6_NAMETABLE
    }

    pub fn get_mapper_id(&self) -> u8 {
        ((self.flags6 >> 4) << 4) | (self.flags7 >> 4)
    }

    pub fn has_battery_backed_ram(&self) -> bool {
        self.flags6 & FLAG6_BATTERY != 0
    }

    pub fn has_four_screen_vram(&self) -> bool {
        self.flags6 & FLAG6_FOUR_SCREEN != 0
    }

    pub fn get_has_trainer(&self) -> bool {
        self.flags6 & FLAG6_TRAINER != 0
    }

    pub fn is_vs_unisystem(&self) -> bool {
        self.flags7 & FLAG7_VS_UNISYSTEM != 0
    }

    pub fn is_playchoice_10(&self) -> bool {
        self.flags7 & FLAG7_PLAYCHOICE_10 != 0
    }

    pub fn is_nes_2_0(&self) -> bool {
        self.flags7 & FLAG7_NES2_SIGNATURE_MASK == FLAG7_NES2_SIGNATURE_VALUE
    }

    pub fn tv_system(&self) -> TvSystem {
        if self.is_nes_2_0() {
            match self.flags10 & FLAG10_TV_SYSTEM_MASK {
                0 => TvSystem::Ntsc,
                2 => TvSystem::Pal,
                1 | 3 => TvSystem::DualCompatible,
                other => TvSystem::Unknown(other),
            }
        } else if self.flags9 & FLAG9_TV_SYSTEM != 0 {
            TvSystem::Pal
        } else {
            TvSystem::Ntsc
        }
    }
}
