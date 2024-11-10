use core::{cell::UnsafeCell, hint::unreachable_unchecked, sync::atomic::AtomicU8};

#[derive(PartialEq, Eq, Clone, Copy, Debug)]
pub(crate) enum Ptr {
    Value1,
    Value2,
}

impl Ptr {
    pub(crate) fn switch(&mut self) {
        *self = match self {
            Ptr::Value1 => Self::Value2,
            Ptr::Value2 => Self::Value1,
        };
    }

    /// SAFETY: Assumes no read bits (lower two) are set
    pub(crate) unsafe fn from_u8_no_read(value: u8) -> Self {
        match value {
            0b000 => Self::Value1,
            0b100 => Self::Value2,
            // SAFETY: unsafe fn. communicated in docs
            _ => unsafe { unreachable_unchecked() },
        }
    }

    /// ignores potentially set read bits.
    /// SAFETY: no bits except the for bottom three can be set
    pub(crate) unsafe fn from_u8_ignore_read(value: u8) -> Self {
        match value & 0b100 {
            0b000 => Self::Value1,
            0b100 => Self::Value2,
            // SAFETY: unsafe fn. communicated in docs
            _ => unsafe { unreachable_unchecked() },
        }
    }
}

#[derive(Debug)]
pub(crate) enum ReadState {
    None,
    Value(Ptr),
}

impl ReadState {
    /// is writing on the passed ptr parameter valid with the current read state?
    #[inline]
    pub(crate) fn can_write(&self, ptr: Ptr) -> bool {
        match self {
            ReadState::None => true,
            ReadState::Value(p) => *p != ptr,
        }
    }

    /// SAFETY: only the read state bits are set
    pub(crate) unsafe fn from_u8_ignore_ptr(value: u8) -> Self {
        match value & 0b011 {
            0b00 => Self::None,
            0b01 => Self::Value(Ptr::Value1),
            0b10 => Self::Value(Ptr::Value2),
            // SAFETY: Internal Library Value only.
            _ => unsafe { unreachable_unchecked() },
        }
    }
}

impl From<Ptr> for ReadState {
    #[inline]
    fn from(value: Ptr) -> Self {
        Self::Value(value)
    }
}

#[derive(Debug)]
pub(crate) struct Shared<T> {
    pub value_1: UnsafeCell<T>,
    pub value_2: UnsafeCell<T>,
    /// bit 0: is value 1 being read
    /// bit 1: is value 2 being read
    /// bit 3: which value should be read next (0: value 1, 1: value 2)
    pub state: AtomicU8,
}

/// SAFETY: Shared not public. Reader and Writer make it safe to use. Same restrictions as RwLock, which allows similar access
unsafe impl<T: Send> Send for Shared<T> {}

/// SAFETY: Shared not public. Reader and Writer make it safe to use. Same restrictions as RwLock, which allows similar access
unsafe impl<T: Send + Sync> Sync for Shared<T> {}

impl<T> Shared<T> {
    pub(crate) fn get_value(&self, ptr: Ptr) -> &UnsafeCell<T> {
        match ptr {
            Ptr::Value1 => &self.value_1,
            Ptr::Value2 => &self.value_2,
        }
    }
}
