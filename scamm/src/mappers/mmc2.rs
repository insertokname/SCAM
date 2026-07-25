use crate::{
    CartridgeConfig, Mapper, MapperError, Mirroring, NametableTarget, ReadMapping, ResetKind,
    WriteMapping,
};

use super::{bank_count, bank_offset, nametable_target};

/// Nintendo MMC2 / PxROM (mapper 9).
#[derive(Debug)]
pub struct M009 {
    config: CartridgeConfig,
    prg_bank: u8,
    chr_fd: [u8; 2],
    chr_fe: [u8; 2],
    latch: [u8; 2],
    mirroring: Mirroring,
}

impl M009 {
    pub fn new(config: CartridgeConfig) -> Result<Self, MapperError> {
        if config.submapper != 0 {
            return Err(MapperError::Unsupported {
                mapper_id: 9,
                submapper: config.submapper,
            });
        }
        if config.prg_rom_len != 0x20000 {
            return Err(MapperError::InvalidConfiguration {
                mapper_id: 9,
                reason: "PxROM requires its documented 128 KiB PRG ROM layout",
            });
        }
        if config.chr_rom_len != 0x20000 || config.total_chr_ram_len() != 0 {
            return Err(MapperError::InvalidConfiguration {
                mapper_id: 9,
                reason: "PxROM requires its documented 128 KiB CHR ROM layout",
            });
        }
        if !matches!(config.total_prg_ram_len(), 0 | 0x2000) {
            return Err(MapperError::InvalidConfiguration {
                mapper_id: 9,
                reason: "PxROM has either no PRG RAM or the PlayChoice 8 KiB RAM circuit",
            });
        }
        let mirroring = config.mirroring;
        Ok(Self {
            config,
            prg_bank: 0,
            chr_fd: [0; 2],
            chr_fe: [0; 2],
            // Nintendo did not specify the power latch. Use Nestopia's
            // deterministic FD selection.
            latch: [0xfd; 2],
            mirroring,
        })
    }

    fn cpu_mapping(&self, address: u16) -> ReadMapping {
        if (0x6000..0x8000).contains(&address) && self.config.total_prg_ram_len() != 0 {
            return ReadMapping::PrgRam(
                (address as usize - 0x6000) % self.config.total_prg_ram_len(),
            );
        }
        if address < 0x8000 {
            return ReadMapping::OpenBus;
        }
        let total = bank_count(self.config.prg_rom_len, 0x2000);
        let slot = (address as usize - 0x8000) / 0x2000;
        let bank = if slot == 0 {
            (self.prg_bank & 0x0f) as usize
        } else {
            total - 4 + slot
        };
        ReadMapping::PrgRom(bank_offset(
            bank,
            0x2000,
            self.config.prg_rom_len,
            address as usize & 0x1fff,
        ))
    }

    fn chr_mapping(&self, address: u16) -> ReadMapping {
        if address >= 0x2000 {
            return ReadMapping::OpenBus;
        }
        let half = usize::from(address >= 0x1000);
        let bank = if self.latch[half] == 0xfd {
            self.chr_fd[half]
        } else {
            self.chr_fe[half]
        };
        ReadMapping::ChrRom(bank_offset(
            (bank & 0x1f) as usize,
            0x1000,
            self.config.chr_rom_len,
            address as usize & 0x0fff,
        ))
    }

    fn update_latch_after_fetch(&mut self, address: u16) {
        match address {
            0x0fd8 => self.latch[0] = 0xfd,
            0x0fe8 => self.latch[0] = 0xfe,
            0x1fd8..=0x1fdf => self.latch[1] = 0xfd,
            0x1fe8..=0x1fef => self.latch[1] = 0xfe,
            _ => {}
        }
    }
}

impl Mapper for M009 {
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
        if (0x6000..0x8000).contains(&address) && self.config.total_prg_ram_len() != 0 {
            return WriteMapping::PrgRam(
                (address as usize - 0x6000) % self.config.total_prg_ram_len(),
            );
        }
        match address {
            0xa000..=0xafff => self.prg_bank = value & 0x0f,
            0xb000..=0xbfff => self.chr_fd[0] = value & 0x1f,
            0xc000..=0xcfff => self.chr_fe[0] = value & 0x1f,
            0xd000..=0xdfff => self.chr_fd[1] = value & 0x1f,
            0xe000..=0xefff => self.chr_fe[1] = value & 0x1f,
            0xf000..=0xffff => {
                self.mirroring = if value & 1 == 0 {
                    Mirroring::Vertical
                } else {
                    Mirroring::Horizontal
                };
            }
            _ => return WriteMapping::Unmapped,
        }
        WriteMapping::Handled
    }
    fn ppu_read(&mut self, address: u16) -> ReadMapping {
        let mapping = self.chr_mapping(address);
        // Hardware completes the trigger fetch from the previous bank, then
        // changes the latch for subsequent pattern fetches.
        self.update_latch_after_fetch(address);
        mapping
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
            self.latch = [0xfd; 2];
            self.prg_bank = 0;
            self.chr_fd = [0; 2];
            self.chr_fe = [0; 2];
            self.mirroring = self.config.mirroring;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TimingMode;

    fn config() -> CartridgeConfig {
        CartridgeConfig {
            mapper_id: 9,
            submapper: 0,
            prg_rom_len: 0x20000,
            chr_rom_len: 0x20000,
            prg_ram_len: 0,
            prg_nvram_len: 0,
            chr_ram_len: 0,
            chr_nvram_len: 0,
            mirroring: Mirroring::Vertical,
            timing: TimingMode::Ntsc,
            has_battery: false,
            has_trainer: false,
            is_nes_2_0: false,
        }
    }

    #[test]
    fn trigger_fetch_uses_old_bank_then_changes_latch() {
        let mut mapper = M009::new(config()).unwrap();
        mapper.cpu_write(0xb000, 3, None);
        mapper.cpu_write(0xc000, 7, None);
        assert_eq!(
            mapper.ppu_read(0x0fd8),
            ReadMapping::ChrRom(3 * 0x1000 + 0x0fd8)
        );
        assert_eq!(mapper.ppu_peek(0), ReadMapping::ChrRom(3 * 0x1000));
    }

    #[test]
    fn fixes_last_three_prg_banks() {
        let mut mapper = M009::new(config()).unwrap();
        mapper.cpu_write(0xa000, 2, None);
        assert_eq!(mapper.cpu_peek(0x8000), ReadMapping::PrgRom(0x4000));
        assert_eq!(mapper.cpu_peek(0xa000), ReadMapping::PrgRom(0x1a000));
        assert_eq!(mapper.cpu_peek(0xe000), ReadMapping::PrgRom(0x1e000));
    }
}
