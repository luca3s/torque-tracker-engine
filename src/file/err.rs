use std::error::Error;
use std::fmt::{Display, Formatter};
use std::io;

use enumflags2::bitflags;

#[derive(Debug)]
pub enum LoadErr {
    CantReadFile,
    Invalid,
    BufferTooShort,
}

impl From<io::Error> for LoadErr {
    fn from(_: io::Error) -> Self {
        Self::CantReadFile
    }
}

impl Display for LoadErr {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}

impl Error for LoadErr {}

/// load was partially successful. These are the defects that are in the now loaded project
#[bitflags]
#[derive(Debug, Clone, Copy)]
#[repr(u8)]
pub enum LoadDefects {
    /// deletes the effect
    UnknownEffect,
    /// replaced with empty text
    InvalidText,
    /// tries to replace with a sane default value
    OutOfBoundsValue,
}
