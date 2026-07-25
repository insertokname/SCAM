#[derive(Clone)]
pub enum CartridgeAccess {
    CpuAccess { address: u16 },
    PpuAccess { address: u16 },
}
