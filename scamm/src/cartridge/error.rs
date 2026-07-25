#[derive(thiserror::Error, Debug)]
pub enum CartridgeParseError {
    #[error("Got an io error while reading a cartridge:\nio error was: {_0}!")]
    IoError(#[from] std::io::Error),
    #[error("Magic number missing at the start of the file. Maybe recieved wrong file type.")]
    MissingMagicNumbersError,
    #[error("Was trying to read {_0} bytes but the data was too short!")]
    NotEnoughBytesError(usize),
    #[error("Unknown mapper id: {_0}!")]
    UnknownMapperIdError(u16),
    #[error("Invalid iNES/NES 2.0 header: {_0}")]
    InvalidHeaderError(#[from] crate::HeaderError),
    #[error("Invalid mapper configuration: {_0}")]
    MapperError(#[from] crate::MapperError),
}
