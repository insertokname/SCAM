use crate::{
    CartridgeConfig, Mapper, MapperError, Mirroring, NametableTarget, ReadMapping, ResetKind,
    WriteMapping,
};

use super::{bank_count, bank_offset, nametable_target};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Variant {
    Compatible,
    Namco175,
    Namco340,
}

/// Namco 175 and Namco 340 (iNES mapper 210).
#[derive(Debug)]
pub struct M210 {
    config: CartridgeConfig,
    variant: Variant,
    chr_banks: [u8; 8],
    prg_banks: [u8; 3],
    prg_ram_enabled: bool,
    mirroring: Mirroring,
}

impl M210 {
    pub fn new(config: CartridgeConfig) -> Result<Self, MapperError> {
        let variant = match config.submapper {
            0 => Variant::Compatible,
            1 => Variant::Namco175,
            2 => Variant::Namco340,
            submapper => {
                return Err(MapperError::Unsupported {
                    mapper_id: 210,
                    submapper,
                });
            }
        };

        if config.prg_rom_len < 0x8000
            || config.prg_rom_len > 0x80000
            || !config.prg_rom_len.is_multiple_of(0x2000)
        {
            return Err(MapperError::InvalidConfiguration {
                mapper_id: 210,
                reason: "Namco 175/340 requires 32-512 KiB PRG ROM in 8 KiB banks",
            });
        }
        if config.chr_rom_len == 0
            || config.chr_rom_len > 0x40000
            || !config.chr_rom_len.is_multiple_of(0x400)
            || config.total_chr_ram_len() != 0
        {
            return Err(MapperError::InvalidConfiguration {
                mapper_id: 210,
                reason: "Namco 175/340 requires up to 256 KiB CHR ROM in 1 KiB banks",
            });
        }
        if matches!(variant, Variant::Compatible | Variant::Namco175)
            && !matches!(config.total_prg_ram_len(), 0 | 0x0800)
        {
            return Err(MapperError::InvalidConfiguration {
                mapper_id: 210,
                reason: "Namco 175 wiring supports either no RAM or one mirrored 2 KiB RAM chip",
            });
        }
        if variant == Variant::Namco175
            && !matches!(
                config.mirroring,
                Mirroring::Horizontal | Mirroring::Vertical
            )
        {
            return Err(MapperError::InvalidConfiguration {
                mapper_id: 210,
                reason: "Namco 175 mirroring must be hardwired horizontal or vertical",
            });
        }
        if variant == Variant::Namco340 && config.is_nes_2_0 && config.total_prg_ram_len() != 0 {
            return Err(MapperError::InvalidConfiguration {
                mapper_id: 210,
                reason: "Namco 340 has no PRG RAM",
            });
        }

        let mirroring = if matches!(variant, Variant::Compatible | Variant::Namco340) {
            Mirroring::Vertical
        } else {
            config.mirroring
        };
        Ok(Self {
            config,
            variant,
            chr_banks: [0; 8],
            prg_banks: [0, 1, 2],
            prg_ram_enabled: false,
            mirroring,
        })
    }

    fn cpu_mapping(&self, address: u16) -> ReadMapping {
        match address {
            0x6000..=0x7fff
                if self.variant != Variant::Namco340
                    && self.prg_ram_enabled
                    && self.config.total_prg_ram_len() != 0 =>
            {
                ReadMapping::PrgRam((address as usize - 0x6000) % self.config.total_prg_ram_len())
            }
            0x8000..=0xdfff => {
                let slot = (address as usize - 0x8000) / 0x2000;
                ReadMapping::PrgRom(bank_offset(
                    self.prg_banks[slot] as usize,
                    0x2000,
                    self.config.prg_rom_len,
                    address as usize & 0x1fff,
                ))
            }
            0xe000..=0xffff => ReadMapping::PrgRom(bank_offset(
                bank_count(self.config.prg_rom_len, 0x2000) - 1,
                0x2000,
                self.config.prg_rom_len,
                address as usize & 0x1fff,
            )),
            _ => ReadMapping::OpenBus,
        }
    }

    fn chr_mapping(&self, address: u16) -> ReadMapping {
        if address >= 0x2000 {
            return ReadMapping::OpenBus;
        }
        let slot = address as usize / 0x400;
        ReadMapping::ChrRom(bank_offset(
            self.chr_banks[slot] as usize,
            0x400,
            self.config.chr_rom_len,
            address as usize & 0x3ff,
        ))
    }

    fn set_namco340_prg_1(&mut self, value: u8) {
        self.prg_banks[0] = value & 0x3f;
        self.mirroring = match value >> 6 {
            0 => Mirroring::OneScreenLower,
            1 => Mirroring::Vertical,
            2 => Mirroring::OneScreenUpper,
            _ => Mirroring::Horizontal,
        };
    }
}

impl Mapper for M210 {
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
        if (0x6000..=0x7fff).contains(&address)
            && self.variant != Variant::Namco340
            && self.prg_ram_enabled
            && self.config.total_prg_ram_len() != 0
        {
            return WriteMapping::PrgRam(
                (address as usize - 0x6000) % self.config.total_prg_ram_len(),
            );
        }

        match address & 0xf800 {
            0x8000..=0xb800 => {
                let slot = (address as usize - 0x8000) / 0x800;
                self.chr_banks[slot] = value;
                WriteMapping::Handled
            }
            0xc000 if self.variant != Variant::Namco340 => {
                self.prg_ram_enabled = value & 1 != 0;
                WriteMapping::Handled
            }
            0xe000 => {
                if self.variant != Variant::Namco175 {
                    self.set_namco340_prg_1(value);
                } else {
                    self.prg_banks[0] = value & 0x3f;
                }
                WriteMapping::Handled
            }
            0xe800 => {
                self.prg_banks[1] = value & 0x3f;
                WriteMapping::Handled
            }
            0xf000 => {
                self.prg_banks[2] = value & 0x3f;
                WriteMapping::Handled
            }
            _ => WriteMapping::Unmapped,
        }
    }

    fn ppu_read(&mut self, address: u16) -> ReadMapping {
        self.chr_mapping(address)
    }

    fn ppu_peek(&self, address: u16) -> ReadMapping {
        self.chr_mapping(address)
    }

    fn ppu_write(&mut self, _: u16, _: u8) -> WriteMapping {
        WriteMapping::Unmapped
    }

    fn nametable_target(&self, address: u16) -> NametableTarget {
        nametable_target(self.mirroring, address)
    }

    fn reset(&mut self, kind: ResetKind) {
        if kind == ResetKind::PowerOn {
            self.chr_banks = [0; 8];
            self.prg_banks = [0, 1, 2];
            self.prg_ram_enabled = false;
            self.mirroring = if self.variant != Variant::Namco175 {
                Mirroring::Vertical
            } else {
                self.config.mirroring
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TimingMode;

    fn config(submapper: u8, prg_ram_len: usize) -> CartridgeConfig {
        CartridgeConfig {
            mapper_id: 210,
            submapper,
            prg_rom_len: 0x80000,
            chr_rom_len: 0x40000,
            prg_ram_len,
            prg_nvram_len: 0,
            chr_ram_len: 0,
            chr_nvram_len: 0,
            mirroring: Mirroring::Horizontal,
            timing: TimingMode::Ntsc,
            has_battery: prg_ram_len != 0,
            has_trainer: false,
            is_nes_2_0: true,
        }
    }

    #[test]
    fn banks_eight_chr_windows_and_three_prg_windows() {
        let mut mapper = M210::new(config(2, 0)).unwrap();
        mapper.cpu_write(0x9abc, 0x23, None);
        mapper.cpu_write(0xe123, 5, None);
        mapper.cpu_write(0xefff, 6, None);
        mapper.cpu_write(0xf456, 7, None);

        assert_eq!(mapper.ppu_peek(0xc00), ReadMapping::ChrRom(0x23 * 0x400));
        assert_eq!(mapper.cpu_peek(0x8000), ReadMapping::PrgRom(5 * 0x2000));
        assert_eq!(mapper.cpu_peek(0xa000), ReadMapping::PrgRom(6 * 0x2000));
        assert_eq!(mapper.cpu_peek(0xc000), ReadMapping::PrgRom(7 * 0x2000));
        assert_eq!(mapper.cpu_peek(0xe000), ReadMapping::PrgRom(63 * 0x2000));
    }

    #[test]
    fn namco340_decodes_all_four_mirroring_modes() {
        let mut mapper = M210::new(config(2, 0)).unwrap();
        let cases = [
            (0x00, NametableTarget::Ciram(0), NametableTarget::Ciram(0)),
            (
                0x40,
                NametableTarget::Ciram(0x400),
                NametableTarget::Ciram(0),
            ),
            (
                0x80,
                NametableTarget::Ciram(0x400),
                NametableTarget::Ciram(0x400),
            ),
            (
                0xc0,
                NametableTarget::Ciram(0),
                NametableTarget::Ciram(0x400),
            ),
        ];
        for (value, expected_at_2400, expected_at_2800) in cases {
            mapper.cpu_write(0xe000, value, None);
            assert_eq!(mapper.nametable_target(0x2400), expected_at_2400);
            assert_eq!(mapper.nametable_target(0x2800), expected_at_2800);
        }

        mapper.cpu_write(0xe000, 0x80, None);
        assert_eq!(
            mapper.nametable_target(0x2000),
            NametableTarget::Ciram(0x400)
        );
        mapper.cpu_write(0xe000, 0xc0, None);
        assert_eq!(mapper.nametable_target(0x2400), NametableTarget::Ciram(0));
    }

    #[test]
    fn namco175_hardwires_mirroring_and_gates_mirrored_ram() {
        let mut mapper = M210::new(config(1, 0x800)).unwrap();
        assert_eq!(mapper.cpu_peek(0x6000), ReadMapping::OpenBus);
        mapper.cpu_write(0xc7ff, 1, None);
        assert_eq!(mapper.cpu_peek(0x6000), ReadMapping::PrgRam(0));
        assert_eq!(mapper.cpu_peek(0x6800), ReadMapping::PrgRam(0));
        assert_eq!(
            mapper.cpu_write(0x7fff, 0xaa, None),
            WriteMapping::PrgRam(0x7ff)
        );

        mapper.cpu_write(0xe000, 0x40, None);
        assert_eq!(mapper.nametable_target(0x2400), NametableTarget::Ciram(0));
        mapper.cpu_write(0xc000, 0, None);
        assert_eq!(mapper.cpu_peek(0x6000), ReadMapping::OpenBus);
    }

    #[test]
    fn submapper_zero_is_combined_without_board_guessing() {
        let mapper_175 = M210::new(config(0, 0x800)).unwrap();
        assert_eq!(mapper_175.variant, Variant::Compatible);
        let mapper_340 = M210::new(config(0, 0)).unwrap();
        assert_eq!(mapper_340.variant, Variant::Compatible);
    }

    #[test]
    fn rejects_ram_on_explicit_namco340() {
        assert!(matches!(
            M210::new(config(2, 0x800)),
            Err(MapperError::InvalidConfiguration { mapper_id: 210, .. })
        ));
    }
}
