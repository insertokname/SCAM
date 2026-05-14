use crate::{
    byte_size,
    hardware::cartrige::{Header, Mapper, cartrige_access::CartrigeAccess},
};

mod mirroring {
    use crate::hardware::cartrige::Header;

    pub(super) fn horizontal(address: u16) -> u16 {
        address & !0x0400
    }

    pub(super) fn vertical(address: u16) -> u16 {
        address & !0x0800
    }

    pub(super) fn from_header(header: &Header, address: u16) -> u16 {
        if header.has_four_screen_vram() {
            address
        } else if header.get_nametable_arrangement() == 0 {
            horizontal(address)
        } else {
            vertical(address)
        }
    }
}

pub(super) struct M000 {
    pub header: Header,
}

impl Mapper for M000 {
    fn new(header: Header) -> Self
    where
        Self: Sized,
    {
        Self { header }
    }

    fn map_read(&mut self, cartrige_access: CartrigeAccess) -> Option<u16> {
        match cartrige_access {
            CartrigeAccess::CpuAccess { address } if address < 0x8000 => None,
            CartrigeAccess::CpuAccess { address } => {
                let offset = address - 0x8000;
                if self.header.prg_rom_size() == 1 {
                    Some(offset & 0x3FFF)
                } else {
                    Some(offset)
                }
            }
            CartrigeAccess::PpuAccess { address } if address < 0x2000 => Some(address),
            CartrigeAccess::PpuAccess { .. } => None,
        }
    }

    fn map_write(&mut self, cartrige_access: CartrigeAccess, _: u8) -> Option<u16> {
        match cartrige_access {
            CartrigeAccess::CpuAccess { .. } => None,
            CartrigeAccess::PpuAccess { address } if address < 0x2000 => {
                if self.header.chr_size == 0 {
                    Some(address)
                } else {
                    None
                }
            }
            CartrigeAccess::PpuAccess { .. } => None,
        }
    }

    fn map_nametable(&self, address: u16) -> u16 {
        mirroring::from_header(&self.header, address)
    }
}

pub(super) struct M002 {
    pub header: Header,
    selected_bank: u8,
}

impl Mapper for M002 {
    fn new(header: Header) -> Self
    where
        Self: Sized,
    {
        Self {
            header,
            selected_bank: 0,
        }
    }

    fn map_read(&mut self, cartrige_access: CartrigeAccess) -> Option<u16> {
        match cartrige_access {
            CartrigeAccess::CpuAccess { address } if address < 0x8000 => None,
            CartrigeAccess::CpuAccess { address } if address < 0xC000 => {
                Some((self.selected_bank) as u16 * byte_size!(16 kb) as u16 + (address & 0x3FFF))
            }
            CartrigeAccess::CpuAccess { address } => Some(
                (self.header.prg_rom_size() - 1) as u16 * byte_size!(16 kb) as u16
                    + (address & 0x3FFF),
            ),
            CartrigeAccess::PpuAccess { address } if address < 0x2000 => Some(address),
            CartrigeAccess::PpuAccess { .. } => None,
        }
    }

    fn map_write(&mut self, cartrige_access: CartrigeAccess, value: u8) -> Option<u16> {
        match cartrige_access {
            CartrigeAccess::CpuAccess { address } if address < 0x8000 => None,
            CartrigeAccess::CpuAccess { .. } => {
                self.selected_bank = value & 0x0F;
                None
            }
            CartrigeAccess::PpuAccess { address } if address < 0x2000 => {
                if self.header.chr_size == 0 {
                    Some(address)
                } else {
                    None
                }
            }
            CartrigeAccess::PpuAccess { .. } => None,
        }
    }

    fn map_nametable(&self, address: u16) -> u16 {
        mirroring::from_header(&self.header, address)
    }
}
