#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BusValue {
    pub value: u8,
    pub driven_mask: u8,
}

impl BusValue {
    pub const fn full(value: u8) -> Self {
        Self {
            value,
            driven_mask: 0xff,
        }
    }

    pub const fn resolve(self, open_bus: u8) -> u8 {
        (self.value & self.driven_mask) | (open_bus & !self.driven_mask)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadMapping {
    PrgRom(usize),
    PrgRam(usize),
    ChrRom(usize),
    ChrRam(usize),
    Value(BusValue),
    OpenBus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteMapping {
    PrgRam(usize),
    ChrRam(usize),
    Handled,
    Unmapped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NametableTarget {
    Ciram(usize),
    CartridgeVram(usize),
    /// A mapper-controlled nametable page backed directly by CHR ROM.
    ChrRom(usize),
    ChrRam(usize),
    MapperRam(usize),
    Fill {
        tile: u8,
        attribute: u8,
    },
    OpenBus,
}
