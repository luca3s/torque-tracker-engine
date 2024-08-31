use std::error::Error;
use std::fmt::{Display, Formatter};
use std::io;

use enumflags2::bitflags;

#[derive(Debug)]
pub(crate) struct TooShortErr;

impl Display for TooShortErr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Buffer too short")
    }
}

impl Error for TooShortErr {}

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

impl From<TooShortErr> for LoadErr {
    fn from(_value: TooShortErr) -> Self {
        LoadErr::BufferTooShort
    }
}

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
