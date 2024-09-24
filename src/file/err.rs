use std::error::Error;
use std::fmt::{Display, Formatter};
use std::io;

#[derive(Debug)]
pub enum LoadErr {
    CantReadFile,
    Invalid,
    BufferTooShort,
    /// A defect handler function returned ControlFlow::Break
    Cancelled,
    IO(io::Error),
}

impl From<io::Error> for LoadErr {
    fn from(err: io::Error) -> Self {
        if err.kind() == io::ErrorKind::UnexpectedEof {
            Self::BufferTooShort
        } else {
            Self::IO(err)
        }
    }
}

impl Display for LoadErr {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}

impl Error for LoadErr {}

/// TODO: https://users.rust-lang.org/t/validation-monad/117894/6
/// 
/// this is a way cleaner and nicer approach. Provice a lot of data about the Error, like position in file, expected value, received value, ...
/// maybe even allow to cancel the parsing via ControlFlow<(), ()>
/// 
/// load was partially successful. These are the defects that are in the now loaded project
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub enum LoadDefect {
    /// deletes the effect
    UnknownEffect,
    /// replaced with empty text
    InvalidText,
    /// tries to replace with a sane default value
    OutOfBoundsValue,
    /// skips loading of the pointed to value
    OutOfBoundsPtr,
}
