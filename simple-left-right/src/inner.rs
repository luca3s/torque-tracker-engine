use core::{
    cell::UnsafeCell, hint::unreachable_unchecked, mem::MaybeUninit, sync::atomic::{self, AtomicU8, Ordering}
};

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

    /// SAFETY: needs to be the internal state u8.
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
    pub access_count: AtomicU8,
}

impl<T> Shared<T> {
    /// initializes everything except for both values.
    pub(crate) fn initialize_state(this: &mut MaybeUninit<Self>) {
        // SAFETY: takes &mut self, so no writing is okay
        unsafe {
            (&raw mut (*this.as_mut_ptr()).access_count).write(AtomicU8::new(1));
            (&raw mut (*this.as_mut_ptr()).state).write(AtomicU8::new(0b000));
        }
    }

    pub(crate) fn get_value(&self, ptr: Ptr) -> &UnsafeCell<T> {
        match ptr {
            Ptr::Value1 => &self.value_1,
            Ptr::Value2 => &self.value_2,
        }
    }

    /// If self is unique increase the count and returns true.
    /// Otherwise returns false.
    ///
    /// If this returns true another smart pointer has to be created otherwise memory will be leaked
    pub(crate) unsafe fn is_unique_with_increase(&self) -> bool {
        // Relaxed taken from std Arc
        let old_access = self.access_count.load(Ordering::Acquire);
        debug_assert!(
            old_access <= 2,
            "at maximum there is one Reader and one Writer"
        );
        if old_access == 1 {
            self.access_count.fetch_add(1, Ordering::Relaxed);
            true
        } else {
            false
        }
    }

    /// decreases the access_count and returns if self should now be dropped.
    ///
    /// | return | needed reaction
    /// |---|---|
    /// | true | self needs to be dropped or memory is leaked |
    /// | false | this ptr to self could now become dangling at any time |
    ///
    /// dropping self when this returns true is safe and needed synchronisation has been done.
    pub(crate) unsafe fn should_drop(&self) -> bool {
        let old_access = self.access_count.fetch_sub(1, Ordering::Release);
        debug_assert!(
            old_access <= 2,
            "at maximum there is one Reader and one Writer"
        );
        if old_access != 1 {
            return false;
        }
        // see std Arc
        atomic::fence(Ordering::Acquire);
        true
    }

    pub(crate) fn is_unique(&self) -> bool {
        let access = self.access_count.load(Ordering::Acquire);
        debug_assert!(access <= 2, "at maximum there is one Reader and one Writer");
        access == 1
    }
}
