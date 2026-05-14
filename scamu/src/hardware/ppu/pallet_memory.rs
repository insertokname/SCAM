use std::fmt::Debug;

use crate::hardware::constants::ppu::PALLET_SIZE;

/// implementation of collor pallets from:
/// https://www.nesdev.org/wiki/PPU_palettes
#[derive(Debug, Default)]
pub struct PalletMemory {
    pallet_memory: [u8; PALLET_SIZE],
}

impl PalletMemory {
    /// `pallet_index` is the pallet to be used and `color_index` is
    /// the color within the pallet to be selected
    pub fn read_index(&self, pallet_index: u16, color_index: u16) -> u8 {
        self.read_address((pallet_index) * 4 + (color_index))
    }

    /// `pallet_index` is the pallet to be used and `color_index` is
    /// the color within the pallet to be selected
    pub fn write_index(&mut self, pallet_index: u16, color_index: u16, value: u8) {
        self.write_address((pallet_index) * 4 + (color_index), value);
    }

    pub fn read_address(&self, address: u16) -> u8 {
        self.pallet_memory[Self::map_pallet_address(address)]
    }

    pub fn write_address(&mut self, address: u16, value: u8) {
        self.pallet_memory[Self::map_pallet_address(address)] = value
    }

    fn map_pallet_address(address: u16) -> usize {
        match (address as usize) % PALLET_SIZE {
            // pallet memory for color 0 is shared between sprites and background
            // https://www.nesdev.org/wiki/PPU_palettes#Palette_RAM
            address if address >= 0x10 && address % 4 == 0 => address - 0x10,
            address => address,
        }
    }
}
