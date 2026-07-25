use thiserror::Error;

const INES_MAGIC: [u8; 4] = *b"NES\x1a";

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum HeaderError {
    #[error("an iNES/NES 2.0 header is exactly 16 bytes")]
    InvalidLength,
    #[error("missing NES<EOF> magic bytes")]
    MissingMagic,
    #[error("ROM size encoded by the header does not fit this platform")]
    RomSizeOverflow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Mirroring {
    Horizontal,
    Vertical,
    FourScreen,
    OneScreenLower,
    OneScreenUpper,
}

impl Mirroring {
    pub fn map_ciram(self, address: u16) -> usize {
        let address = (address as usize).wrapping_sub(0x2000) & 0x0fff;
        match self {
            Self::Horizontal => ((address >> 1) & 0x0400) | (address & 0x03ff),
            Self::Vertical => address & 0x07ff,
            Self::FourScreen => address,
            Self::OneScreenLower => address & 0x03ff,
            Self::OneScreenUpper => 0x0400 | (address & 0x03ff),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TimingMode {
    Ntsc,
    Pal,
    MultiRegion,
    Dendy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TvSystem {
    Ntsc,
    Pal,
    DualCompatible,
    Unknown(u8),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CartridgeConfig {
    pub mapper_id: u16,
    pub submapper: u8,
    pub prg_rom_len: usize,
    pub chr_rom_len: usize,
    pub prg_ram_len: usize,
    pub prg_nvram_len: usize,
    pub chr_ram_len: usize,
    pub chr_nvram_len: usize,
    pub mirroring: Mirroring,
    pub timing: TimingMode,
    pub has_battery: bool,
    pub has_trainer: bool,
    pub is_nes_2_0: bool,
}

impl CartridgeConfig {
    pub fn total_prg_ram_len(&self) -> usize {
        self.prg_ram_len.saturating_add(self.prg_nvram_len)
    }

    pub fn total_chr_ram_len(&self) -> usize {
        self.chr_ram_len.saturating_add(self.chr_nvram_len)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InesHeader {
    raw: [u8; 16],
    config: CartridgeConfig,
}

impl InesHeader {
    pub fn parse(bytes: &[u8]) -> Result<Self, HeaderError> {
        let raw: [u8; 16] = bytes.try_into().map_err(|_| HeaderError::InvalidLength)?;
        if raw[..4] != INES_MAGIC {
            return Err(HeaderError::MissingMagic);
        }

        let is_nes_2_0 = raw[7] & 0x0c == 0x08;
        let mapper_id = (raw[7] as u16 & 0xf0) | (raw[6] as u16 >> 4);
        let mapper_id = if is_nes_2_0 {
            mapper_id | ((raw[8] as u16 & 0x0f) << 8)
        } else {
            mapper_id
        };
        let submapper = if is_nes_2_0 { raw[8] >> 4 } else { 0 };

        let prg_rom_len = if is_nes_2_0 {
            decode_nes2_rom_size(raw[4], raw[9] & 0x0f, 16 * 1024)?
        } else {
            raw[4] as usize * 16 * 1024
        };
        let chr_rom_len = if is_nes_2_0 {
            decode_nes2_rom_size(raw[5], raw[9] >> 4, 8 * 1024)?
        } else {
            raw[5] as usize * 8 * 1024
        };

        let (prg_ram_len, prg_nvram_len, chr_ram_len, chr_nvram_len) = if is_nes_2_0 {
            (
                decode_nes2_ram_size(raw[10] & 0x0f)?,
                decode_nes2_ram_size(raw[10] >> 4)?,
                decode_nes2_ram_size(raw[11] & 0x0f)?,
                decode_nes2_ram_size(raw[11] >> 4)?,
            )
        } else {
            // iNES 1.0 byte 8 value zero means "unspecified", not that every
            // board contains an 8 KiB RAM.  Only mapper families whose common
            // submapper-0 wiring decodes $6000-$7FFF receive the historical
            // compatibility default.  NROM receives it only for the
            // battery-backed Family BASIC-style fallback.
            let prg_memory_len = if raw[8] != 0 {
                raw[8] as usize * 8 * 1024
            } else {
                legacy_ines_prg_ram_len(mapper_id, raw[6] & 0x02 != 0)
            };
            let (prg_ram_len, prg_nvram_len) = if raw[6] & 0x02 != 0 {
                (0, prg_memory_len)
            } else {
                (prg_memory_len, 0)
            };
            (
                prg_ram_len,
                prg_nvram_len,
                if chr_rom_len == 0 { 8 * 1024 } else { 0 },
                0,
            )
        };

        let mirroring = if raw[6] & 0x08 != 0 {
            Mirroring::FourScreen
        } else if raw[6] & 0x01 != 0 {
            Mirroring::Vertical
        } else {
            Mirroring::Horizontal
        };
        let timing = if is_nes_2_0 {
            match raw[12] & 0x03 {
                0 => TimingMode::Ntsc,
                1 => TimingMode::Pal,
                2 => TimingMode::MultiRegion,
                _ => TimingMode::Dendy,
            }
        } else if raw[9] & 1 != 0 {
            TimingMode::Pal
        } else {
            TimingMode::Ntsc
        };

        Ok(Self {
            raw,
            config: CartridgeConfig {
                mapper_id,
                submapper,
                prg_rom_len,
                chr_rom_len,
                prg_ram_len,
                prg_nvram_len,
                chr_ram_len,
                chr_nvram_len,
                mirroring,
                timing,
                has_battery: raw[6] & 0x02 != 0,
                has_trainer: raw[6] & 0x04 != 0,
                is_nes_2_0,
            },
        })
    }

    pub fn raw(&self) -> &[u8; 16] {
        &self.raw
    }

    pub fn config(&self) -> &CartridgeConfig {
        &self.config
    }

    pub fn into_config(self) -> CartridgeConfig {
        self.config
    }

    // Compatibility accessors used by scamu's existing public API.
    pub fn prg_rom_size(&self) -> u8 {
        self.raw[4]
    }

    pub fn prg_chr_size(&self) -> u8 {
        self.raw[5]
    }

    pub fn prg_rom_size_bytes(&self) -> usize {
        self.config.prg_rom_len
    }

    pub fn chr_rom_size_bytes(&self) -> usize {
        self.config.chr_rom_len
    }

    pub fn prg_ram_size_bytes(&self) -> usize {
        self.config.total_prg_ram_len()
    }

    pub fn get_nametable_arrangement(&self) -> u8 {
        self.raw[6] & 1
    }

    pub fn get_mapper_id(&self) -> u16 {
        self.config.mapper_id
    }

    pub fn submapper(&self) -> u8 {
        self.config.submapper
    }

    pub fn has_battery_backed_ram(&self) -> bool {
        self.config.has_battery
    }

    pub fn has_four_screen_vram(&self) -> bool {
        self.config.mirroring == Mirroring::FourScreen
    }

    pub fn get_has_trainer(&self) -> bool {
        self.config.has_trainer
    }

    pub fn is_vs_unisystem(&self) -> bool {
        self.raw[7] & 1 != 0
    }

    pub fn is_playchoice_10(&self) -> bool {
        self.raw[7] & 2 != 0
    }

    pub fn is_nes_2_0(&self) -> bool {
        self.config.is_nes_2_0
    }

    pub fn tv_system(&self) -> TvSystem {
        match self.config.timing {
            TimingMode::Ntsc => TvSystem::Ntsc,
            TimingMode::Pal => TvSystem::Pal,
            TimingMode::MultiRegion => TvSystem::DualCompatible,
            TimingMode::Dendy => TvSystem::Unknown(3),
        }
    }
}

fn legacy_ines_prg_ram_len(mapper_id: u16, has_battery: bool) -> usize {
    match mapper_id {
        // Family BASIC and Namco 175 use RAM chips smaller than iNES 1.0 can
        // encode. Their battery bit is the useful legacy discriminator.
        0 if has_battery => 0x1000,
        210 if has_battery => 0x0800,
        // ASIC boards whose original iNES definition commonly includes a
        // directly mapped $6000-$7FFF RAM window.  Serial EEPROM-only boards
        // (notably mapper 16) are deliberately absent.
        1 | 4 | 5 | 9 | 18 | 19 | 23 | 69 => 0x2000,
        _ => 0,
    }
}

fn decode_nes2_rom_size(lsb: u8, msb: u8, unit: usize) -> Result<usize, HeaderError> {
    if msb != 0x0f {
        return (((msb as usize) << 8) | lsb as usize)
            .checked_mul(unit)
            .ok_or(HeaderError::RomSizeOverflow);
    }
    let exponent = lsb >> 2;
    let multiplier = ((lsb & 0x03) as usize * 2) + 1;
    1usize
        .checked_shl(exponent as u32)
        .and_then(|base| base.checked_mul(multiplier))
        .ok_or(HeaderError::RomSizeOverflow)
}

fn decode_nes2_ram_size(shift: u8) -> Result<usize, HeaderError> {
    if shift == 0 {
        Ok(0)
    } else {
        64usize
            .checked_shl(shift as u32)
            .ok_or(HeaderError::RomSizeOverflow)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header() -> [u8; 16] {
        let mut header = [0; 16];
        header[..4].copy_from_slice(&INES_MAGIC);
        header
    }

    #[test]
    fn ines_mapper_nibbles_have_the_correct_order() {
        let mut raw = header();
        raw[6] = 0x21;
        raw[7] = 0xa0;
        let parsed = InesHeader::parse(&raw).unwrap();
        assert_eq!(parsed.get_mapper_id(), 0xa2);
        assert_eq!(parsed.config.mirroring, Mirroring::Vertical);
    }

    #[test]
    fn parses_nes2_mapper_submapper_and_ram() {
        let mut raw = header();
        raw[4] = 2;
        raw[5] = 1;
        raw[6] = 0x30;
        raw[7] = 0x48;
        raw[8] = 0x51;
        raw[10] = 0x87;
        raw[11] = 0x07;
        raw[12] = 3;
        let parsed = InesHeader::parse(&raw).unwrap();
        assert_eq!(parsed.config.mapper_id, 0x143);
        assert_eq!(parsed.config.submapper, 5);
        assert_eq!(parsed.config.prg_ram_len, 8192);
        assert_eq!(parsed.config.prg_nvram_len, 16384);
        assert_eq!(parsed.config.chr_ram_len, 8192);
        assert_eq!(parsed.config.timing, TimingMode::Dendy);
    }

    #[test]
    fn ines_chr_zero_provisions_chr_ram() {
        let parsed = InesHeader::parse(&header()).unwrap();
        assert_eq!(parsed.config.chr_rom_len, 0);
        assert_eq!(parsed.config.chr_ram_len, 8192);
    }

    #[test]
    fn ines_mapper_zero_does_not_invent_prg_ram_from_unspecified_byte_eight() {
        let mut raw = header();
        raw[4] = 1;
        raw[5] = 1;
        let parsed = InesHeader::parse(&raw).unwrap();
        assert_eq!(parsed.config.mapper_id, 0);
        assert_eq!(parsed.config.total_prg_ram_len(), 0);
    }

    #[test]
    fn ines_battery_memory_is_classified_as_nonvolatile() {
        let mut raw = header();
        raw[4] = 2;
        raw[5] = 1;
        raw[6] = 0x02;
        raw[8] = 2;
        let parsed = InesHeader::parse(&raw).unwrap();
        assert_eq!(parsed.config.prg_ram_len, 0);
        assert_eq!(parsed.config.prg_nvram_len, 0x4000);
    }

    #[test]
    fn unspecified_legacy_ram_defaults_are_mapper_aware() {
        for (mapper, battery, expected) in [
            (0, false, 0),
            (0, true, 0x1000),
            (1, false, 0x2000),
            (2, false, 0),
            (3, false, 0),
            (4, false, 0x2000),
            (5, false, 0x2000),
            (7, false, 0),
            (9, false, 0x2000),
            (11, false, 0),
            (16, true, 0),
            (18, false, 0x2000),
            (19, false, 0x2000),
            (23, false, 0x2000),
            (66, false, 0),
            (69, false, 0x2000),
            (71, false, 0),
            (79, false, 0),
            (87, false, 0),
            (206, false, 0),
            (210, true, 0x0800),
        ] {
            assert_eq!(
                legacy_ines_prg_ram_len(mapper, battery),
                expected,
                "mapper {mapper}"
            );
        }
    }
}
