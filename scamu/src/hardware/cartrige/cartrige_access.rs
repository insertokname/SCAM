#[derive(Clone)]
pub enum CartrigeAccess {
    CpuAccess { address: u16 },
    PpuAccess { address: u16 },
}
