use crate::{
    CartridgeConfig, Mapper, MapperError, Mirroring, NametableTarget, ReadMapping, ResetKind,
    WriteMapping,
};

use super::{bank_offset, map_chr_read, nametable_target};

/// American Video Entertainment NINA-03/NINA-06 (iNES mapper 79).
#[derive(Debug)]
pub struct M079 {
    config: CartridgeConfig,
    register: u8,
}

impl M079 {
    pub fn new(config: CartridgeConfig) -> Result<Self, MapperError> {
        if config.submapper != 0 {
            return Err(MapperError::Unsupported {
                mapper_id: 79,
                submapper: config.submapper,
            });
        }
        if !matches!(config.prg_rom_len, 0x8000 | 0x10000) {
            return Err(MapperError::InvalidConfiguration {
                mapper_id: 79,
                reason: "NINA-03/NINA-06 requires 32 KiB or 64 KiB PRG ROM",
            });
        }
        if config.chr_rom_len == 0
            || config.chr_rom_len > 0x10000
            || !config.chr_rom_len.is_multiple_of(0x2000)
            || config.total_chr_ram_len() != 0
        {
            return Err(MapperError::InvalidConfiguration {
                mapper_id: 79,
                reason: "NINA-03/NINA-06 requires 8-64 KiB CHR ROM in 8 KiB banks",
            });
        }
        if config.total_prg_ram_len() != 0 {
            return Err(MapperError::InvalidConfiguration {
                mapper_id: 79,
                reason: "NINA-03/NINA-06 has no PRG RAM",
            });
        }
        if !matches!(
            config.mirroring,
            Mirroring::Horizontal | Mirroring::Vertical
        ) {
            return Err(MapperError::InvalidConfiguration {
                mapper_id: 79,
                reason: "NINA-03/NINA-06 mirroring must be hardwired horizontal or vertical",
            });
        }

        Ok(Self {
            config,
            register: 0,
        })
    }

    fn register_address(address: u16) -> bool {
        (0x4100..=0x5fff).contains(&address) && address & 0xe100 == 0x4100
    }

    fn prg_bank(&self) -> usize {
        ((self.register >> 3) & 1) as usize
    }

    fn chr_bank(&self) -> usize {
        (self.register & 7) as usize
    }

    fn cpu_mapping(&self, address: u16) -> ReadMapping {
        if address >= 0x8000 {
            ReadMapping::PrgRom(bank_offset(
                self.prg_bank(),
                0x8000,
                self.config.prg_rom_len,
                address as usize & 0x7fff,
            ))
        } else {
            ReadMapping::OpenBus
        }
    }
}

impl Mapper for M079 {
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
        if Self::register_address(address) {
            self.register = value;
            WriteMapping::Handled
        } else {
            WriteMapping::Unmapped
        }
    }

    fn ppu_read(&mut self, address: u16) -> ReadMapping {
        map_chr_read(&self.config, address, self.chr_bank())
    }

    fn ppu_peek(&self, address: u16) -> ReadMapping {
        map_chr_read(&self.config, address, self.chr_bank())
    }

    fn ppu_write(&mut self, _: u16, _: u8) -> WriteMapping {
        WriteMapping::Unmapped
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Mirroring, TimingMode};

    fn config() -> CartridgeConfig {
        CartridgeConfig {
            mapper_id: 79,
            submapper: 0,
            prg_rom_len: 0x10000,
            chr_rom_len: 0x10000,
            prg_ram_len: 0,
            prg_nvram_len: 0,
            chr_ram_len: 0,
            chr_nvram_len: 0,
            mirroring: Mirroring::Horizontal,
            timing: TimingMode::Ntsc,
            has_battery: false,
            has_trainer: false,
            is_nes_2_0: true,
        }
    }

    #[test]
    fn control_register_selects_prg_and_chr() {
        let mut mapper = M079::new(config()).unwrap();
        mapper.cpu_write(0x4100, 0x0d, None);
        assert_eq!(mapper.cpu_peek(0x8000), ReadMapping::PrgRom(0x8000));
        assert_eq!(mapper.ppu_peek(0), ReadMapping::ChrRom(5 * 0x2000));
    }

    #[test]
    fn register_decode_requires_a8_and_mirrors_every_0x200() {
        let mut mapper = M079::new(config()).unwrap();
        assert_eq!(mapper.cpu_write(0x4200, 0x0f, None), WriteMapping::Unmapped);
        assert_eq!(mapper.cpu_peek(0x8000), ReadMapping::PrgRom(0));
        assert_eq!(mapper.cpu_write(0x5f7a, 0x0f, None), WriteMapping::Handled);
        assert_eq!(mapper.cpu_peek(0x8000), ReadMapping::PrgRom(0x8000));
        assert_eq!(mapper.ppu_peek(0), ReadMapping::ChrRom(7 * 0x2000));
    }

    #[test]
    fn fixed_header_mirroring_is_not_changed_by_bank_value() {
        let mut mapper = M079::new(config()).unwrap();
        mapper.cpu_write(0x4100, 0xff, None);
        assert_eq!(
            mapper.nametable_target(0x2800),
            NametableTarget::Ciram(0x400)
        );
    }

    #[test]
    fn boundaries_resets_and_negative_features_are_exact() {
        let mut mapper = M079::new(config()).unwrap();
        assert_eq!(mapper.cpu_peek(0x7fff), ReadMapping::OpenBus);
        assert_eq!(mapper.cpu_peek(0x8000), ReadMapping::PrgRom(0));
        assert_eq!(mapper.cpu_peek(0xffff), ReadMapping::PrgRom(0x7fff));
        assert_eq!(mapper.ppu_peek(0x1fff), ReadMapping::ChrRom(0x1fff));
        assert_eq!(mapper.ppu_peek(0x2000), ReadMapping::OpenBus);
        assert_eq!(mapper.cpu_write(0x40ff, 0xff, None), WriteMapping::Unmapped);
        assert_eq!(mapper.cpu_write(0x41ff, 0x0f, None), WriteMapping::Handled);
        assert_eq!(mapper.cpu_write(0x6000, 0, None), WriteMapping::Unmapped);
        assert_eq!(mapper.ppu_write(0, 0), WriteMapping::Unmapped);
        assert!(!mapper.irq_pending());
        assert_eq!(mapper.expansion_audio_sample(), 0.0);

        mapper.reset(ResetKind::ResetButton);
        assert_eq!(mapper.cpu_peek(0x8000), ReadMapping::PrgRom(0x8000));
        assert_eq!(mapper.ppu_peek(0), ReadMapping::ChrRom(7 * 0x2000));
        mapper.reset(ResetKind::PowerOn);
        assert_eq!(mapper.cpu_peek(0x8000), ReadMapping::PrgRom(0));
        assert_eq!(mapper.ppu_peek(0), ReadMapping::ChrRom(0));

        let mut ram = config();
        ram.prg_ram_len = 0x2000;
        assert!(matches!(
            M079::new(ram),
            Err(MapperError::InvalidConfiguration { mapper_id: 79, .. })
        ));
        let mut variant = config();
        variant.submapper = 1;
        assert!(matches!(
            M079::new(variant),
            Err(MapperError::Unsupported {
                mapper_id: 79,
                submapper: 1
            })
        ));
    }
}
