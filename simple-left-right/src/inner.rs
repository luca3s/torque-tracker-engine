use core::{cell::UnsafeCell, hint::unreachable_unchecked, sync::atomic::AtomicU8};

#[derive(PartialEq, Eq, Clone, Copy, Debug)]
pub enum Ptr {
    Value1,
    Value2,
}

impl Ptr {
    #[inline]
    pub fn switch(&mut self) {
        *self = match self {
            Ptr::Value1 => Self::Value2,
            Ptr::Value2 => Self::Value1,
        };
    }
}

impl From<u8> for Ptr {
    #[inline]
    fn from(value: u8) -> Self {
        match value & 0b100 {
            0b000 => Self::Value1,
            0b100 => Self::Value2,
            _ => unsafe { unreachable_unchecked() },
        }
    }
}

#[derive(Debug)]
pub enum ReadState {
    None,
    Value(Ptr),
    /// Is written before loading the ptr value and then instantly overwritten with the specific ptr value
    /// This makes sure the Writer doesn't swap and load between loading the ptr and setting the read
    Both,
}

impl ReadState {
    /// is writing on the passed ptr parameter valid with the current read state?
    #[inline]
    pub fn can_write(&self, ptr: Ptr) -> bool {
        match self {
            ReadState::None => true,
            ReadState::Value(p) => *p != ptr,
            ReadState::Both => false,
        }
    }
}

impl From<Ptr> for ReadState {
    #[inline]
    fn from(value: Ptr) -> Self {
        Self::Value(value)
    }
}

impl From<u8> for ReadState {
    #[inline]
    fn from(value: u8) -> Self {
        match value & 0b011 {
            0b00 => Self::None,
            0b01 => Self::Value(Ptr::Value1),
            0b10 => Self::Value(Ptr::Value2),
            0b11 => Self::Both,
            _ => unsafe { unreachable_unchecked() },
        }
    }
}

pub struct Shared<T> {
    pub value_1: UnsafeCell<T>,
    pub value_2: UnsafeCell<T>,
    /// bit 0: is value 1 being read
    /// bit 1: is value 2 being read
    /// bit 3: which value should be read next (0: value 1, 1: value 2)
    pub state: AtomicU8,
}

unsafe impl<T: Send> Send for Shared<T> {}
unsafe impl<T: Send + Sync> Sync for Shared<T> {}

impl<T> Shared<T> {
    pub fn get_value(&self, ptr: Ptr) -> &UnsafeCell<T> {
        match ptr {
            Ptr::Value1 => &self.value_1,
            Ptr::Value2 => &self.value_2,
        }
    }
}
