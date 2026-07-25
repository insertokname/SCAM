use crate::{
    CartridgeConfig, Mapper, MapperError, Mirroring, NametableTarget, ReadMapping, ResetKind,
    WriteMapping,
};

use super::{bank_count, bank_offset, map_chr_read, map_chr_write, nametable_target};

/// Camerica/Codemasters BF9093 and BF9097 (iNES mapper 71).
#[derive(Debug)]
pub struct M071 {
    config: CartridgeConfig,
    prg_bank: usize,
    controlled_mirroring: Option<Mirroring>,
}

impl M071 {
    pub fn new(config: CartridgeConfig) -> Result<Self, MapperError> {
        if config.submapper > 1 {
            return Err(MapperError::Unsupported {
                mapper_id: 71,
                submapper: config.submapper,
            });
        }
        if !matches!(config.prg_rom_len, 0x20000 | 0x40000) {
            return Err(MapperError::InvalidConfiguration {
                mapper_id: 71,
                reason: "BF9093/BF9097 requires 128 KiB or 256 KiB PRG ROM (64 KiB is Mapper 232)",
            });
        }
        if config.submapper == 1 && config.prg_rom_len != 0x20000 {
            return Err(MapperError::InvalidConfiguration {
                mapper_id: 71,
                reason: "BF9097 exposes only three PRG bank bits and requires 128 KiB PRG ROM",
            });
        }
        if config.chr_rom_len != 0 || config.total_chr_ram_len() != 0x2000 {
            return Err(MapperError::InvalidConfiguration {
                mapper_id: 71,
                reason: "BF9093/BF9097 requires exactly 8 KiB CHR RAM and no CHR ROM",
            });
        }
        if config.total_prg_ram_len() != 0 {
            return Err(MapperError::InvalidConfiguration {
                mapper_id: 71,
                reason: "BF9093/BF9097 has no PRG RAM",
            });
        }
        if config.submapper == 0
            && !matches!(
                config.mirroring,
                Mirroring::Horizontal | Mirroring::Vertical
            )
        {
            return Err(MapperError::InvalidConfiguration {
                mapper_id: 71,
                reason: "BF9093 mirroring must be hardwired horizontal or vertical",
            });
        }

        let controlled_mirroring = (config.submapper == 1).then_some(Mirroring::OneScreenLower);
        Ok(Self {
            config,
            prg_bank: 0,
            controlled_mirroring,
        })
    }

    fn cpu_mapping(&self, address: u16) -> ReadMapping {
        match address {
            0x8000..=0xbfff => ReadMapping::PrgRom(bank_offset(
                self.prg_bank,
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

impl Mapper for M071 {
    fn config(&self) -> &CartridgeConfig {
        &self.config
    }

    fn cpu_read(&mut self, address: u16) -> ReadMapping {
        self.cpu_mapping(address)
    }

    fn cpu_peek(&self, address: u16) -> ReadMapping {
        self.cpu_mapping(address)
    }

    fn cpu_write(&mut self, address: u16, value: u8, _: Option<u8>) -> WriteMapping {
        match address {
            0x8000..=0x9fff if self.config.submapper == 1 => {
                self.controlled_mirroring = Some(if value & 0x10 == 0 {
                    Mirroring::OneScreenLower
                } else {
                    Mirroring::OneScreenUpper
                });
                WriteMapping::Handled
            }
            0xc000..=0xffff => {
                self.prg_bank = usize::from(
                    value
                        & if self.config.submapper == 1 {
                            0x07
                        } else {
                            0x0f
                        },
                );
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
        nametable_target(
            self.controlled_mirroring.unwrap_or(self.config.mirroring),
            address,
        )
    }

    fn reset(&mut self, kind: ResetKind) {
        if kind == ResetKind::PowerOn {
            self.prg_bank = 0;
            self.controlled_mirroring =
                (self.config.submapper == 1).then_some(Mirroring::OneScreenLower);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TimingMode;

    fn config(submapper: u8) -> CartridgeConfig {
        CartridgeConfig {
            mapper_id: 71,
            submapper,
            prg_rom_len: if submapper == 1 { 0x20000 } else { 0x40000 },
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
    fn switches_16k_low_bank_and_fixes_last_bank() {
        let mut mapper = M071::new(config(0)).unwrap();
        assert_eq!(mapper.cpu_write(0xc123, 5, None), WriteMapping::Handled);
        assert_eq!(mapper.cpu_peek(0x8000), ReadMapping::PrgRom(5 * 0x4000));
        assert_eq!(mapper.cpu_peek(0xc000), ReadMapping::PrgRom(15 * 0x4000));
    }

    #[test]
    fn bf9093_does_not_decode_fire_hawk_register() {
        let mut mapper = M071::new(config(0)).unwrap();
        assert_eq!(mapper.cpu_write(0x9000, 0x10, None), WriteMapping::Unmapped);
        assert_eq!(mapper.nametable_target(0x2800), NametableTarget::Ciram(0));
    }

    #[test]
    fn fire_hawk_controls_one_screen_page_only_below_a000() {
        let mut mapper = M071::new(config(1)).unwrap();
        mapper.cpu_write(0x9fff, 0x10, None);
        assert_eq!(
            mapper.nametable_target(0x2000),
            NametableTarget::Ciram(0x400)
        );
        assert_eq!(mapper.cpu_write(0xa000, 0, None), WriteMapping::Unmapped);
        assert_eq!(mapper.cpu_peek(0x8000), ReadMapping::PrgRom(0));
    }

    #[test]
    fn rejects_unknown_submapper() {
        assert!(matches!(
            M071::new(config(2)),
            Err(MapperError::Unsupported {
                mapper_id: 71,
                submapper: 2
            })
        ));
    }

    #[test]
    fn rejects_64k_as_mapper_232() {
        let mut cfg = config(0);
        cfg.prg_rom_len = 0x10000;
        assert!(matches!(
            M071::new(cfg),
            Err(MapperError::InvalidConfiguration { mapper_id: 71, .. })
        ));
    }

    #[test]
    fn each_chip_variant_ignores_unwired_prg_bank_bits() {
        let mut bf9093 = M071::new(config(0)).unwrap();
        bf9093.cpu_write(0xc000, 0xf5, None);
        assert_eq!(bf9093.cpu_peek(0x8000), ReadMapping::PrgRom(5 * 0x4000));

        let mut bf9097 = M071::new(config(1)).unwrap();
        bf9097.cpu_write(0xc000, 0xfd, None);
        assert_eq!(bf9097.cpu_peek(0x8000), ReadMapping::PrgRom(5 * 0x4000));
    }
}
