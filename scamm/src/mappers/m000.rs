use crate::{
    CartridgeConfig, Mapper, MapperError, NametableTarget, ReadMapping, ResetKind, WriteMapping,
};

use super::{map_chr_read, map_chr_write, nametable_target};

/// NROM (iNES mapper 0).
#[derive(Debug)]
pub struct M000 {
    config: CartridgeConfig,
    prg_ram_enabled: bool,
}

impl M000 {
    pub fn new(config: CartridgeConfig) -> Result<Self, MapperError> {
        if config.submapper != 0 {
            return Err(MapperError::Unsupported {
                mapper_id: 0,
                submapper: config.submapper,
            });
        }
        if !matches!(config.prg_rom_len, 0x2000 | 0x4000 | 0x8000) {
            return Err(MapperError::InvalidConfiguration {
                mapper_id: 0,
                reason: "mapper 0 requires a validated 8 KiB, 16 KiB, or 32 KiB PRG ROM variant",
            });
        }
        if !matches!(config.total_prg_ram_len(), 0 | 0x0800 | 0x1000) {
            return Err(MapperError::InvalidConfiguration {
                mapper_id: 0,
                reason: "mapper 0 PRG RAM is only validated for the 2 KiB and 4 KiB Family BASIC variants",
            });
        }
        let chr_rom = config.chr_rom_len;
        let chr_ram = config.total_chr_ram_len();
        if !matches!((chr_rom, chr_ram), (0x2000, 0) | (0, 0x2000) | (0, 0)) {
            return Err(MapperError::InvalidConfiguration {
                mapper_id: 0,
                reason: "mapper 0 requires exactly one unbanked 8 KiB CHR ROM or CHR RAM image",
            });
        }
        Ok(Self {
            config,
            prg_ram_enabled: true,
        })
    }

    fn cpu_mapping(&self, address: u16) -> ReadMapping {
        if address < 0x6000 {
            ReadMapping::OpenBus
        } else if address < 0x8000 && self.prg_ram_enabled && self.config.total_prg_ram_len() != 0 {
            ReadMapping::PrgRam((address as usize - 0x6000) % self.config.total_prg_ram_len())
        } else if address >= 0x8000 {
            ReadMapping::PrgRom((address as usize - 0x8000) % self.config.prg_rom_len)
        } else {
            ReadMapping::OpenBus
        }
    }
}

impl Mapper for M000 {
    fn config(&self) -> &CartridgeConfig {
        &self.config
    }

    fn cpu_read(&mut self, address: u16) -> ReadMapping {
        self.cpu_mapping(address)
    }

    fn cpu_peek(&self, address: u16) -> ReadMapping {
        self.cpu_mapping(address)
    }

    fn cpu_write(&mut self, address: u16, _: u8, _: Option<u8>) -> WriteMapping {
        if (0x6000..0x8000).contains(&address)
            && self.prg_ram_enabled
            && self.config.total_prg_ram_len() != 0
        {
            WriteMapping::PrgRam((address as usize - 0x6000) % self.config.total_prg_ram_len())
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
        nametable_target(self.config.mirroring, address)
    }

    fn set_prg_ram_enabled(&mut self, enabled: bool) {
        self.prg_ram_enabled = enabled;
    }

    fn reset(&mut self, _: ResetKind) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Mirroring, TimingMode};
    use proptest::prelude::*;

    fn config(prg_rom_len: usize, chr_rom_len: usize) -> CartridgeConfig {
        CartridgeConfig {
            mapper_id: 0,
            submapper: 0,
            prg_rom_len,
            chr_rom_len,
            prg_ram_len: 0,
            prg_nvram_len: 0,
            chr_ram_len: usize::from(chr_rom_len == 0) * 0x2000,
            chr_nvram_len: 0,
            mirroring: Mirroring::Horizontal,
            timing: TimingMode::Ntsc,
            has_battery: false,
            has_trainer: false,
            is_nes_2_0: false,
        }
    }

    #[test]
    fn mirrors_nrom_128() {
        let mapper = M000::new(config(0x4000, 0x2000)).unwrap();
        assert_eq!(mapper.cpu_peek(0x8000), ReadMapping::PrgRom(0));
        assert_eq!(mapper.cpu_peek(0xc000), ReadMapping::PrgRom(0));
        assert_eq!(mapper.cpu_peek(0xffff), ReadMapping::PrgRom(0x3fff));
    }

    #[test]
    fn chr_ram_is_writable() {
        let mut mapper = M000::new(config(0x8000, 0)).unwrap();
        assert_eq!(mapper.ppu_read(0x1234), ReadMapping::ChrRam(0x1234));
        assert_eq!(mapper.ppu_write(0x1234, 0xaa), WriteMapping::ChrRam(0x1234));
    }

    #[test]
    fn chr_rom_is_fixed_read_only_and_outside_pattern_space_is_open_bus() {
        let mut mapper = M000::new(config(0x8000, 0x2000)).unwrap();
        for address in 0..0x2000 {
            assert_eq!(
                mapper.ppu_peek(address),
                ReadMapping::ChrRom(address as usize)
            );
            assert_eq!(mapper.ppu_write(address, 0xff), WriteMapping::Unmapped);
        }
        assert_eq!(mapper.ppu_peek(0x2000), ReadMapping::OpenBus);
    }

    #[test]
    fn every_cpu_address_maps_to_the_exact_unbanked_nrom_image() {
        for prg_rom_len in [0x2000, 0x4000, 0x8000] {
            let mapper = M000::new(config(prg_rom_len, 0x2000)).unwrap();
            for address in 0x8000..=0xffff {
                assert_eq!(
                    mapper.cpu_peek(address),
                    ReadMapping::PrgRom((address as usize - 0x8000) % prg_rom_len),
                    "wrong mapping for {prg_rom_len:#x}-byte image at {address:#06x}"
                );
            }
        }
    }

    #[test]
    fn nes2_can_explicitly_declare_no_chr_storage() {
        let mut cfg = config(0x4000, 0);
        cfg.chr_ram_len = 0;
        cfg.is_nes_2_0 = true;
        let mut mapper = M000::new(cfg).unwrap();
        for address in 0..0x2000 {
            assert_eq!(mapper.ppu_peek(address), ReadMapping::OpenBus);
            assert_eq!(mapper.ppu_write(address, 0xaa), WriteMapping::Unmapped);
        }
    }

    #[test]
    fn rejects_unknown_variants_and_banked_chr_images() {
        for submapper in 1..=15 {
            let mut cfg = config(0x8000, 0x2000);
            cfg.submapper = submapper;
            assert!(matches!(
                M000::new(cfg),
                Err(MapperError::Unsupported {
                    mapper_id: 0,
                    submapper: rejected
                }) if rejected == submapper
            ));
        }

        assert!(matches!(
            M000::new(config(0x8000, 0x4000)),
            Err(MapperError::InvalidConfiguration { mapper_id: 0, .. })
        ));

        assert!(matches!(
            M000::new(config(0xb800, 0x2000)),
            Err(MapperError::InvalidConfiguration { mapper_id: 0, .. })
        ));

        let mut cfg = config(0x8000, 0x2000);
        cfg.prg_ram_len = 0x2000;
        assert!(matches!(
            M000::new(cfg),
            Err(MapperError::InvalidConfiguration { mapper_id: 0, .. })
        ));
    }

    #[test]
    fn family_basic_ram_mirrors_declared_size_and_external_switch_isolates_it() {
        for ram_len in [0x800, 0x1000] {
            let mut cfg = config(0x8000, 0x2000);
            cfg.prg_nvram_len = ram_len;
            cfg.has_battery = true;
            let mut mapper = M000::new(cfg).unwrap();
            for address in 0x6000..0x8000 {
                let offset = (address as usize - 0x6000) % ram_len;
                assert_eq!(mapper.cpu_peek(address), ReadMapping::PrgRam(offset));
                assert_eq!(
                    mapper.cpu_write(address, 0x5a, None),
                    WriteMapping::PrgRam(offset)
                );
            }
            mapper.set_prg_ram_enabled(false);
            assert_eq!(mapper.cpu_peek(0x6000), ReadMapping::OpenBus);
            assert_eq!(mapper.cpu_write(0x6000, 0, None), WriteMapping::Unmapped);
            mapper.reset(ResetKind::ResetButton);
            assert_eq!(mapper.cpu_peek(0x7fff), ReadMapping::OpenBus);
            mapper.set_prg_ram_enabled(true);
            assert_eq!(mapper.cpu_peek(0x6000), ReadMapping::PrgRam(0));
        }
    }

    #[test]
    fn fixed_mirroring_reset_and_negative_features_never_change() {
        for mirroring in [Mirroring::Horizontal, Mirroring::Vertical] {
            let mut cfg = config(0x4000, 0x2000);
            cfg.mirroring = mirroring;
            let mut mapper = M000::new(cfg).unwrap();
            let before =
                [0x2000, 0x2400, 0x2800, 0x2c00].map(|address| mapper.nametable_target(address));
            let expected = match mirroring {
                Mirroring::Horizontal => [0, 0, 0x400, 0x400],
                Mirroring::Vertical => [0, 0x400, 0, 0x400],
                _ => unreachable!(),
            };
            assert_eq!(before, expected.map(NametableTarget::Ciram));
            for address in 0x8000..=0xffff {
                mapper.cpu_write(address, address as u8, Some(!address as u8));
                mapper.clock_cpu();
                mapper.notify_ppu_address(address & 0x1fff, address as u64, true);
                assert!(!mapper.irq_pending());
                assert_eq!(mapper.expansion_audio_sample(), 0.0);
            }
            mapper.reset(ResetKind::ResetButton);
            assert_eq!(
                before,
                [0x2000, 0x2400, 0x2800, 0x2c00].map(|address| mapper.nametable_target(address))
            );
        }
    }

    proptest! {
        #[test]
        fn writes_and_resets_never_change_fixed_prg_or_chr_mapping(
            prg_index in 0usize..3,
            operations in prop::collection::vec((any::<u16>(), any::<u8>(), any::<bool>()), 0..512),
        ) {
            let prg_len = [0x2000, 0x4000, 0x8000][prg_index];
            let mut mapper = M000::new(config(prg_len, 0x2000)).unwrap();
            for (raw_address, value, reset) in operations {
                let address = 0x8000 | (raw_address & 0x7fff);
                prop_assert_eq!(mapper.cpu_write(address, value, Some(!value)), WriteMapping::Unmapped);
                if reset { mapper.reset(ResetKind::ResetButton); }
                prop_assert_eq!(mapper.cpu_peek(address), ReadMapping::PrgRom((address as usize - 0x8000) % prg_len));
                let ppu_address = raw_address & 0x1fff;
                prop_assert_eq!(mapper.ppu_peek(ppu_address), ReadMapping::ChrRom(ppu_address as usize));
                prop_assert!(!mapper.irq_pending());
            }
        }
    }
}
