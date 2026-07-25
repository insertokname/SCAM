use crate::{
    CartridgeConfig, Mapper, MapperError, NametableTarget, ReadMapping, ResetKind, WriteMapping,
};

use super::{bank_count, bank_offset, map_chr_read, map_chr_write, nametable_target};

/// UxROM (iNES mapper 2).
#[derive(Debug)]
pub struct M002 {
    config: CartridgeConfig,
    selected_bank: usize,
}

impl M002 {
    pub fn new(config: CartridgeConfig) -> Result<Self, MapperError> {
        if config.submapper > 2 {
            return Err(MapperError::Unsupported {
                mapper_id: 2,
                submapper: config.submapper,
            });
        }
        if config.prg_rom_len < 0x8000
            || config.prg_rom_len > 0x80000
            || !config.prg_rom_len.is_multiple_of(0x4000)
            || !config.prg_rom_len.is_power_of_two()
        {
            return Err(MapperError::InvalidConfiguration {
                mapper_id: 2,
                reason: "UxROM requires a power-of-two count of 16 KiB PRG banks from 32 KiB through the documented 512 KiB extension",
            });
        }
        if config.chr_rom_len != 0 || config.total_chr_ram_len() != 0x2000 {
            return Err(MapperError::InvalidConfiguration {
                mapper_id: 2,
                reason: "UxROM requires exactly 8 KiB CHR RAM and no CHR ROM",
            });
        }
        if !matches!(config.total_prg_ram_len(), 0 | 0x2000) {
            return Err(MapperError::InvalidConfiguration {
                mapper_id: 2,
                reason: "UxROM supports only the documented optional 8 KiB NES 2.0 PRG RAM extension",
            });
        }
        Ok(Self {
            config,
            selected_bank: 0,
        })
    }

    fn cpu_mapping(&self, address: u16) -> ReadMapping {
        match address {
            0x6000..=0x7fff if self.config.total_prg_ram_len() != 0 => {
                ReadMapping::PrgRam((address as usize - 0x6000) % self.config.total_prg_ram_len())
            }
            0x8000..=0xbfff => ReadMapping::PrgRom(bank_offset(
                self.selected_bank,
                0x4000,
                self.config.prg_rom_len,
                address as usize & 0x3fff,
            )),
            0xc000..=0xffff => ReadMapping::PrgRom(bank_offset(
                bank_count(self.config.prg_rom_len, 0x4000) - 1,
                0x4000,
                self.config.prg_rom_len,
                address as usize & 0x3fff,
            )),
            _ => ReadMapping::OpenBus,
        }
    }
}

impl Mapper for M002 {
    fn config(&self) -> &CartridgeConfig {
        &self.config
    }

    fn cpu_read(&mut self, address: u16) -> ReadMapping {
        self.cpu_mapping(address)
    }

    fn cpu_peek(&self, address: u16) -> ReadMapping {
        self.cpu_mapping(address)
    }

    fn cpu_write(&mut self, address: u16, value: u8, rom_bus_value: Option<u8>) -> WriteMapping {
        match address {
            0x6000..=0x7fff if self.config.total_prg_ram_len() != 0 => {
                WriteMapping::PrgRam((address as usize - 0x6000) % self.config.total_prg_ram_len())
            }
            0x8000..=0xffff => {
                let has_bus_conflicts = self.config.submapper != 1;
                let value = if has_bus_conflicts {
                    value & rom_bus_value.unwrap_or(0xff)
                } else {
                    value
                };
                self.selected_bank = value as usize;
                WriteMapping::Handled
            }
            _ => WriteMapping::Unmapped,
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
        nametable_target(self.config.mirroring, address)
    }

    fn reset(&mut self, kind: ResetKind) {
        if kind == ResetKind::PowerOn {
            self.selected_bank = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Mirroring, TimingMode};

    fn config(submapper: u8) -> CartridgeConfig {
        CartridgeConfig {
            mapper_id: 2,
            submapper,
            prg_rom_len: 0x40000,
            chr_rom_len: 0,
            prg_ram_len: 0,
            prg_nvram_len: 0,
            chr_ram_len: 0x2000,
            chr_nvram_len: 0,
            mirroring: Mirroring::Vertical,
            timing: TimingMode::Ntsc,
            has_battery: false,
            has_trainer: false,
            is_nes_2_0: true,
        }
    }

    #[test]
    fn switches_low_and_fixes_last_bank() {
        let mut mapper = M002::new(config(1)).unwrap();
        mapper.cpu_write(0x8000, 3, Some(0));
        assert_eq!(mapper.cpu_peek(0x8000), ReadMapping::PrgRom(0xc000));
        assert_eq!(mapper.cpu_peek(0xc000), ReadMapping::PrgRom(0x3c000));
    }

    #[test]
    fn submapper_zero_models_and_bus_conflicts() {
        let mut mapper = M002::new(config(0)).unwrap();
        mapper.cpu_write(0x8000, 7, Some(2));
        assert_eq!(mapper.cpu_peek(0x8000), ReadMapping::PrgRom(0x8000));
    }

    #[test]
    fn rejects_unknown_bus_conflict_variants() {
        assert!(matches!(
            M002::new(config(3)),
            Err(MapperError::Unsupported {
                mapper_id: 2,
                submapper: 3
            })
        ));
    }

    #[test]
    fn explicit_nes2_ram_requires_the_documented_eight_kib_extension() {
        let mut valid = config(0);
        valid.prg_ram_len = 0x2000;
        let mapper = M002::new(valid).unwrap();
        assert_eq!(mapper.cpu_peek(0x6000), ReadMapping::PrgRam(0));
        assert_eq!(mapper.cpu_peek(0x7fff), ReadMapping::PrgRam(0x1fff));

        let mut invalid = config(0);
        invalid.prg_ram_len = 0x1000;
        assert!(matches!(
            M002::new(invalid),
            Err(MapperError::InvalidConfiguration { mapper_id: 2, .. })
        ));
    }
}
