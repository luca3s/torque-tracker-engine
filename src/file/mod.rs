use std::array;

use err::TooShortErr;

pub mod err;
pub mod impulse_format;

pub(crate) struct FileReader<'a> {
    buf: &'a [u8],
    position: usize,
}

impl<'a> FileReader<'a> {
    pub const fn new(buf: &'a [u8]) -> Self {
        FileReader { buf, position: 0 }
    }

    pub fn get(&mut self) -> Result<u8, TooShortErr> {
        let out = self.buf.get(self.position).copied().ok_or(TooShortErr);
        self.position += 1;
        out
    }

    pub fn get_num<const N: usize>(&mut self) -> Result<[u8; N], TooShortErr> {
        self.require_remaining(N)?;

        let out = array::from_fn(|idx| self.buf[self.position + idx]);
        self.position += N;
        Ok(out)
    }

    /// Ok(None) if the string includes invalid utf8 characters
    pub fn get_c_str(&mut self, max_len: usize) -> Result<Option<String>, TooShortErr> {
        self.require_remaining(max_len)?;

        let str = self.buf[self.position..self.position + max_len]
            .split(|b| *b == 0)
            .next()
            .unwrap()
            .to_vec();
        let str = String::from_utf8(str).ok();
        Ok(str)
    }

    /// assumes little endiannes in the file
    pub fn get_u16(&mut self) -> Result<u16, TooShortErr> {
        Ok(u16::from_le_bytes(self.get_num()?))
    }

    /// assumes little endianness in the file
    pub fn get_u32(&mut self) -> Result<u32, TooShortErr> {
        Ok(u32::from_le_bytes(self.get_num()?))
    }

    /// asserts that self is at least as long as other and that the overlapping part is the same
    pub fn assert_eq(&mut self, other: &[u8]) -> Result<bool, TooShortErr> {
        self.require_remaining(other.len())?;

        let out = Ok(&self.buf[self.position..self.position + other.len()] == other);
        self.position += other.len();
        out
    }

    pub fn skip(&mut self, num: usize) -> Result<(), TooShortErr> {
        self.require_remaining(num)?;

        self.position += num;
        Ok(())
    }

    pub const fn require_remaining(&self, rem: usize) -> Result<(), TooShortErr> {
        match self.buf.len() > self.position + rem {
            true => Ok(()),
            false => Err(TooShortErr),
        }
    }

    pub const fn require_overall(&self, len: usize) -> Result<(), TooShortErr> {
        match self.buf.len() > len {
            true => Ok(()),
            false => Err(TooShortErr),
        }
    }
}
