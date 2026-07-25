use crate::{CartridgeConfig, Mirroring, NametableTarget, ReadMapping, WriteMapping};

pub(crate) fn bank_count(len: usize, bank_size: usize) -> usize {
    len.div_ceil(bank_size).max(1)
}

pub(crate) fn bank_offset(bank: usize, bank_size: usize, len: usize, inner: usize) -> usize {
    let bank = bank % bank_count(len, bank_size);
    (bank * bank_size + inner % bank_size) % len.max(1)
}

pub(crate) fn map_chr_read(config: &CartridgeConfig, address: u16, bank: usize) -> ReadMapping {
    if address >= 0x2000 {
        return ReadMapping::OpenBus;
    }
    if config.chr_rom_len != 0 {
        ReadMapping::ChrRom(bank_offset(
            bank,
            0x2000,
            config.chr_rom_len,
            address as usize,
        ))
    } else if config.total_chr_ram_len() != 0 {
        ReadMapping::ChrRam(bank_offset(
            bank,
            0x2000,
            config.total_chr_ram_len(),
            address as usize,
        ))
    } else {
        ReadMapping::OpenBus
    }
}

pub(crate) fn map_chr_write(config: &CartridgeConfig, address: u16, bank: usize) -> WriteMapping {
    if address < 0x2000 && config.chr_rom_len == 0 && config.total_chr_ram_len() != 0 {
        WriteMapping::ChrRam(bank_offset(
            bank,
            0x2000,
            config.total_chr_ram_len(),
            address as usize,
        ))
    } else {
        WriteMapping::Unmapped
    }
}

pub(crate) fn nametable_target(mirroring: Mirroring, address: u16) -> NametableTarget {
    let offset = mirroring.map_ciram(address);
    if mirroring == Mirroring::FourScreen {
        NametableTarget::CartridgeVram(offset)
    } else {
        NametableTarget::Ciram(offset)
    }
}
