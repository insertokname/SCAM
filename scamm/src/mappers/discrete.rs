use crate::{
    CartridgeConfig, Mapper, MapperError, Mirroring, NametableTarget, ReadMapping, ResetKind,
    WriteMapping,
};

use super::{bank_count, bank_offset, map_chr_read, map_chr_write, nametable_target};

fn fixed_prg(config: &CartridgeConfig, address: u16) -> ReadMapping {
    match address {
        0x6000..=0x7fff if config.total_prg_ram_len() != 0 => {
            ReadMapping::PrgRam((address as usize - 0x6000) % config.total_prg_ram_len())
        }
        0x8000..=0xffff => {
            ReadMapping::PrgRom((address as usize - 0x8000) % config.prg_rom_len.max(1))
        }
        _ => ReadMapping::OpenBus,
    }
}

fn fixed_prg_write(config: &CartridgeConfig, address: u16) -> Option<WriteMapping> {
    ((0x6000..0x8000).contains(&address) && config.total_prg_ram_len() != 0)
        .then(|| WriteMapping::PrgRam((address as usize - 0x6000) % config.total_prg_ram_len()))
}

/// CNROM (mapper 3): fixed PRG and switchable 8 KiB CHR.
#[derive(Debug)]
pub struct M003 {
    config: CartridgeConfig,
    chr_bank: usize,
}

impl M003 {
    pub fn new(config: CartridgeConfig) -> Result<Self, MapperError> {
        if config.submapper > 2 {
            return Err(MapperError::Unsupported {
                mapper_id: 3,
                submapper: config.submapper,
            });
        }
        if !matches!(config.prg_rom_len, 0x4000 | 0x8000)
            || !(0x2000..=0x20000).contains(&config.chr_rom_len)
            || !config.chr_rom_len.is_multiple_of(0x2000)
            || config.total_chr_ram_len() != 0
        {
            return Err(MapperError::InvalidConfiguration {
                mapper_id: 3,
                reason: "CNROM requires 16/32 KiB PRG ROM and whole 8 KiB CHR ROM banks up to 128 KiB",
            });
        }
        if !matches!(config.total_prg_ram_len(), 0 | 0x0800) {
            return Err(MapperError::InvalidConfiguration {
                mapper_id: 3,
                reason: "CNROM supports only the documented optional 2 KiB mirrored PRG RAM variant",
            });
        }
        Ok(Self {
            config,
            chr_bank: 0,
        })
    }
}

impl Mapper for M003 {
    fn config(&self) -> &CartridgeConfig {
        &self.config
    }
    fn cpu_read(&mut self, address: u16) -> ReadMapping {
        fixed_prg(&self.config, address)
    }
    fn cpu_peek(&self, address: u16) -> ReadMapping {
        fixed_prg(&self.config, address)
    }
    fn cpu_write(&mut self, address: u16, value: u8, rom: Option<u8>) -> WriteMapping {
        if let Some(mapping) = fixed_prg_write(&self.config, address) {
            return mapping;
        }
        if address >= 0x8000 {
            self.chr_bank = (if self.config.submapper == 1 {
                value
            } else {
                value & rom.unwrap_or(0xff)
            } & 0x0f) as usize;
            WriteMapping::Handled
        } else {
            WriteMapping::Unmapped
        }
    }
    fn ppu_read(&mut self, address: u16) -> ReadMapping {
        map_chr_read(&self.config, address, self.chr_bank)
    }
    fn ppu_peek(&self, address: u16) -> ReadMapping {
        map_chr_read(&self.config, address, self.chr_bank)
    }
    fn ppu_write(&mut self, address: u16, _: u8) -> WriteMapping {
        map_chr_write(&self.config, address, self.chr_bank)
    }
    fn nametable_target(&self, address: u16) -> NametableTarget {
        nametable_target(self.config.mirroring, address)
    }
    fn reset(&mut self, kind: ResetKind) {
        if kind == ResetKind::PowerOn {
            self.chr_bank = 0;
        }
    }
}

/// AxROM (mapper 7): switchable 32 KiB PRG, CHR RAM and one-screen mirroring.
#[derive(Debug)]
pub struct M007 {
    config: CartridgeConfig,
    register: u8,
}

impl M007 {
    pub fn new(config: CartridgeConfig) -> Result<Self, MapperError> {
        if config.submapper > 2 {
            return Err(MapperError::Unsupported {
                mapper_id: 7,
                submapper: config.submapper,
            });
        }
        if !(0x8000..=0x80000).contains(&config.prg_rom_len)
            || !config.prg_rom_len.is_multiple_of(0x8000)
        {
            return Err(MapperError::InvalidConfiguration {
                mapper_id: 7,
                reason: "AxROM requires whole 32 KiB PRG banks up to the documented 512 KiB extension",
            });
        }
        if config.chr_rom_len != 0 || config.total_chr_ram_len() != 0x2000 {
            return Err(MapperError::InvalidConfiguration {
                mapper_id: 7,
                reason: "AxROM requires exactly 8 KiB CHR RAM and no CHR ROM",
            });
        }
        if config.total_prg_ram_len() != 0 {
            return Err(MapperError::InvalidConfiguration {
                mapper_id: 7,
                reason: "AxROM has no PRG RAM",
            });
        }
        Ok(Self {
            config,
            register: 0,
        })
    }

    fn cpu_mapping(&self, address: u16) -> ReadMapping {
        if address >= 0x8000 {
            ReadMapping::PrgRom(bank_offset(
                (self.register & 0x0f) as usize,
                0x8000,
                self.config.prg_rom_len,
                address as usize & 0x7fff,
            ))
        } else {
            fixed_prg(&self.config, address)
        }
    }
}

impl Mapper for M007 {
    fn config(&self) -> &CartridgeConfig {
        &self.config
    }
    fn cpu_read(&mut self, address: u16) -> ReadMapping {
        self.cpu_mapping(address)
    }
    fn cpu_peek(&self, address: u16) -> ReadMapping {
        self.cpu_mapping(address)
    }
    fn cpu_write(&mut self, address: u16, value: u8, rom: Option<u8>) -> WriteMapping {
        if let Some(mapping) = fixed_prg_write(&self.config, address) {
            return mapping;
        }
        if address >= 0x8000 {
            self.register = if self.config.submapper == 2 {
                value & rom.unwrap_or(0xff)
            } else {
                value
            };
            WriteMapping::Handled
        } else {
            WriteMapping::Unmapped
        }
    }
    fn ppu_read(&mut self, address: u16) -> ReadMapping {
        map_chr_read(&self.config, address, 0)
    }
    fn ppu_peek(&self, address: u16) -> ReadMapping {
        map_chr_read(&self.config, address, 0)
    }
    fn ppu_write(&mut self, address: u16, _: u8) -> WriteMapping {
        map_chr_write(&self.config, address, 0)
    }
    fn nametable_target(&self, address: u16) -> NametableTarget {
        let mirroring = if self.register & 0x10 == 0 {
            Mirroring::OneScreenLower
        } else {
            Mirroring::OneScreenUpper
        };
        nametable_target(mirroring, address)
    }
    fn reset(&mut self, kind: ResetKind) {
        if kind == ResetKind::PowerOn {
            self.register = 0;
        }
    }
}

macro_rules! impl_gnrom_like {
    ($name:ident, $mapper:literal, $prg_shift:literal, $prg_mask:literal, $chr_shift:literal, $chr_mask:literal) => {
        #[derive(Debug)]
        pub struct $name {
            config: CartridgeConfig,
            register: u8,
        }

        impl $name {
            pub fn new(config: CartridgeConfig) -> Result<Self, MapperError> {
                if ($mapper == 66 || $mapper == 11) && config.submapper != 0 {
                    return Err(MapperError::Unsupported {
                        mapper_id: $mapper,
                        submapper: config.submapper,
                    });
                }
                if config.prg_rom_len == 0 || config.chr_rom_len == 0 {
                    return Err(MapperError::InvalidConfiguration {
                        mapper_id: $mapper,
                        reason: "mapper requires PRG ROM and CHR ROM",
                    });
                }
                if $mapper == 66
                    && (!(0x8000..=0x80000).contains(&config.prg_rom_len)
                        || !config.prg_rom_len.is_multiple_of(0x8000)
                        || !config.prg_rom_len.is_power_of_two()
                        || !(0x2000..=0x20000).contains(&config.chr_rom_len)
                        || !config.chr_rom_len.is_multiple_of(0x2000)
                        || !config.chr_rom_len.is_power_of_two()
                        || config.total_chr_ram_len() != 0
                        || config.total_prg_ram_len() != 0)
                {
                    return Err(MapperError::InvalidConfiguration {
                        mapper_id: 66,
                        reason: "GxROM requires power-of-two 32 KiB PRG banks up to the 512 KiB oversize wiring, power-of-two 8 KiB CHR banks up to 128 KiB, and no RAM",
                    });
                }
                if $mapper == 11
                    && (!(0x8000..=0x20000).contains(&config.prg_rom_len)
                        || !config.prg_rom_len.is_multiple_of(0x8000)
                        || !(0x2000..=0x20000).contains(&config.chr_rom_len)
                        || !config.chr_rom_len.is_multiple_of(0x2000)
                        || config.total_chr_ram_len() != 0
                        || config.total_prg_ram_len() != 0)
                {
                    return Err(MapperError::InvalidConfiguration {
                        mapper_id: 11,
                        reason: "Color Dreams requires 32 KiB PRG banks up to 128 KiB, 8 KiB CHR banks up to 128 KiB, and no RAM",
                    });
                }
                Ok(Self {
                    config,
                    register: 0,
                })
            }
            fn cpu_mapping(&self, address: u16) -> ReadMapping {
                if address >= 0x8000 {
                    let bank = if $mapper == 66 {
                        let connected = bank_count(self.config.prg_rom_len, 0x8000)
                            .next_power_of_two()
                            - 1;
                        (self.register as usize >> 4) & connected
                    } else {
                        ((self.register >> $prg_shift) & $prg_mask) as usize
                    };
                    ReadMapping::PrgRom(bank_offset(
                        bank,
                        0x8000,
                        self.config.prg_rom_len,
                        address as usize & 0x7fff,
                    ))
                } else {
                    fixed_prg(&self.config, address)
                }
            }
        }

        impl Mapper for $name {
            fn config(&self) -> &CartridgeConfig {
                &self.config
            }
            fn cpu_read(&mut self, address: u16) -> ReadMapping {
                self.cpu_mapping(address)
            }
            fn cpu_peek(&self, address: u16) -> ReadMapping {
                self.cpu_mapping(address)
            }
            fn cpu_write(&mut self, address: u16, value: u8, rom: Option<u8>) -> WriteMapping {
                if let Some(mapping) = fixed_prg_write(&self.config, address) {
                    return mapping;
                }
                if address >= 0x8000 {
                    self.register = value & rom.unwrap_or(0xff);
                    WriteMapping::Handled
                } else {
                    WriteMapping::Unmapped
                }
            }
            fn ppu_read(&mut self, address: u16) -> ReadMapping {
                let bank = if $mapper == 66 {
                    self.register as usize
                        & (bank_count(self.config.chr_rom_len, 0x2000).next_power_of_two() - 1)
                } else {
                    ((self.register >> $chr_shift) & $chr_mask) as usize
                };
                map_chr_read(
                    &self.config,
                    address,
                    bank,
                )
            }
            fn ppu_peek(&self, address: u16) -> ReadMapping {
                let bank = if $mapper == 66 {
                    self.register as usize
                        & (bank_count(self.config.chr_rom_len, 0x2000).next_power_of_two() - 1)
                } else {
                    ((self.register >> $chr_shift) & $chr_mask) as usize
                };
                map_chr_read(
                    &self.config,
                    address,
                    bank,
                )
            }
            fn ppu_write(&mut self, address: u16, _: u8) -> WriteMapping {
                let bank = if $mapper == 66 {
                    self.register as usize
                        & (bank_count(self.config.chr_rom_len, 0x2000).next_power_of_two() - 1)
                } else {
                    ((self.register >> $chr_shift) & $chr_mask) as usize
                };
                map_chr_write(
                    &self.config,
                    address,
                    bank,
                )
            }
            fn nametable_target(&self, address: u16) -> NametableTarget {
                nametable_target(self.config.mirroring, address)
            }
            fn reset(&mut self, kind: ResetKind) {
                if kind == ResetKind::PowerOn {
                    self.register = 0;
                }
            }
        }
    };
}

// Color Dreams: bits 0-1 PRG, bits 4-7 CHR.
impl_gnrom_like!(M011, 11, 0, 0x03, 4, 0x0f);
// GxROM: bits 4-5 PRG, bits 0-1 CHR.
impl_gnrom_like!(M066, 66, 4, 0x03, 0, 0x03);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TimingMode;

    fn config(mapper_id: u16, submapper: u8, prg: usize, chr: usize) -> CartridgeConfig {
        CartridgeConfig {
            mapper_id,
            submapper,
            prg_rom_len: prg,
            chr_rom_len: chr,
            prg_ram_len: 0,
            prg_nvram_len: 0,
            chr_ram_len: usize::from(chr == 0) * 0x2000,
            chr_nvram_len: 0,
            mirroring: Mirroring::Horizontal,
            timing: TimingMode::Ntsc,
            has_battery: false,
            has_trainer: false,
            is_nes_2_0: true,
        }
    }

    #[test]
    fn cnrom_switches_chr_with_conflict() {
        let mut mapper = M003::new(config(3, 2, 0x8000, 0x8000)).unwrap();
        mapper.cpu_write(0x8000, 3, Some(1));
        assert_eq!(mapper.ppu_peek(0), ReadMapping::ChrRom(0x2000));
    }

    #[test]
    fn axrom_switches_prg_and_one_screen_page() {
        let mut mapper = M007::new(config(7, 1, 0x40000, 0)).unwrap();
        mapper.cpu_write(0x8000, 0x12, None);
        assert_eq!(mapper.cpu_peek(0x8000), ReadMapping::PrgRom(0x10000));
        assert_eq!(
            mapper.nametable_target(0x2000),
            NametableTarget::Ciram(0x400)
        );
    }

    #[test]
    fn gxrom_register_selects_both_memories() {
        let mut mapper = M066::new(config(66, 0, 0x20000, 0x8000)).unwrap();
        mapper.cpu_write(0x8000, 0x31, Some(0xff));
        assert_eq!(mapper.cpu_peek(0x8000), ReadMapping::PrgRom(0x18000));
        assert_eq!(mapper.ppu_peek(0), ReadMapping::ChrRom(0x2000));
    }

    #[test]
    fn cnrom_supports_the_documented_four_bit_oversize_latch() {
        let mut mapper = M003::new(config(3, 1, 0x8000, 0x20000)).unwrap();
        mapper.cpu_write(0x8000, 0x1f, None);
        assert_eq!(mapper.ppu_peek(0), ReadMapping::ChrRom(15 * 0x2000));
    }

    #[test]
    fn discrete_submapper_numbers_are_fail_closed() {
        assert!(matches!(
            M003::new(config(3, 3, 0x8000, 0x8000)),
            Err(MapperError::Unsupported {
                mapper_id: 3,
                submapper: 3
            })
        ));
        assert!(matches!(
            M007::new(config(7, 3, 0x40000, 0)),
            Err(MapperError::Unsupported {
                mapper_id: 7,
                submapper: 3
            })
        ));
        assert!(matches!(
            M066::new(config(66, 1, 0x20000, 0x8000)),
            Err(MapperError::Unsupported {
                mapper_id: 66,
                submapper: 1
            })
        ));
    }

    #[test]
    fn axrom_submapper_two_applies_and_bus_conflicts() {
        let mut mapper = M007::new(config(7, 2, 0x40000, 0)).unwrap();
        mapper.cpu_write(0x8000, 0x17, Some(0x12));
        assert_eq!(mapper.cpu_peek(0x8000), ReadMapping::PrgRom(2 * 0x8000));
        assert_eq!(
            mapper.nametable_target(0x2000),
            NametableTarget::Ciram(0x400)
        );
    }

    #[test]
    fn gxrom_supports_documented_oversize_latch_wiring() {
        let mut mapper = M066::new(config(66, 0, 0x80000, 0x20000)).unwrap();
        mapper.cpu_write(0x8000, 0xf9, Some(0xff));
        assert_eq!(mapper.cpu_peek(0x8000), ReadMapping::PrgRom(15 * 0x8000));
        assert_eq!(mapper.ppu_peek(0), ReadMapping::ChrRom(9 * 0x2000));
    }
}
