use core::{
    cell::UnsafeCell,
    hint::assert_unchecked,
    mem::MaybeUninit,
    sync::atomic::{self, AtomicU8, Ordering},
};

#[derive(Clone, Copy, Debug)]
#[repr(u8)]
pub(crate) enum Ptr {
    Value1 = 0,
    Value2 = 0b10000,
}

impl Ptr {
    pub(crate) fn switch(&mut self) {
        *self = match self {
            Ptr::Value1 => Self::Value2,
            Ptr::Value2 => Self::Value1,
        };
    }
}

#[repr(transparent)]
struct State(u8);

impl State {
    const NOREAD_VALUE1_1ACCESS: Self = Self(0b00001);
    const NOREAD_MASK: u8 = 0b10011;
    const READ_MASK: u8 = 0b01100;

    // this should be inlined to have the assert_unchecked be useful
    #[inline]
    fn access_count(self) -> u8 {
        // mask out the top bits
        let access = self.0 & 0b00011;
        // SAFETY: The library only supports one reader and one writer at a time. This is checked in the is_unique functions of Shared
        // the compiler already knows that it is 3 at max. we constrict it further
        unsafe { assert_unchecked(access <= 2); }
        access
    }

    fn read_ptr(self) -> Ptr {
        // mask out everything except the read ptr
        // (self.0 & 0b10000) as Ptr
        if self.0 & 0b10000 == 0 {
            Ptr::Value1
        } else {
            Ptr::Value2
        }
    }

    fn with_read(self, ptr: Ptr) -> Self {
        match ptr {
            Ptr::Value1 => Self(self.0 | 0b00100),
            Ptr::Value2 => Self(self.0 | 0b01000),
        }
    }

    fn can_write(self, ptr: Ptr) -> bool {
        #[expect(clippy::match_like_matches_macro)] // i think it's more readable like this
        match (self.0 & Self::READ_MASK, ptr) {
            (0b0100, Ptr::Value1) => false,
            (0b1000, Ptr::Value2) => false,
            _ => true
        }
    }
}

#[derive(Debug)]
pub(crate) struct Shared<T> {
    pub value_1: UnsafeCell<T>,
    pub value_2: UnsafeCell<T>,
    /// ### Bits from low to high
    /// | bit | meaning |
    /// |---|---|
    /// | 0-1 | how many Reader or Writer exist (max 2) |
    /// | 2 | is value 1 being read |
    /// | 3 | is value 2 being read |
    /// | 4 | which value should be read next (0: value 1, 1: value 2) |
    ///
    /// This mixed use doesn't lead to more contention because there are only two threads max.
    ///
    /// Access count is in the lower bits, so that fetch_add and fetch_sub still work for that purpose.
    /// Locking and Unlocking is done via bitand / or and fetch_update anyways
    state: AtomicU8,
}

impl<T> Shared<T> {
    pub(crate) fn lock_read(&self) -> Ptr {
        let result = self
            .state
            .fetch_update(Ordering::Relaxed, Ordering::Acquire, |value| {
                Some(State(value).with_read(State(value).read_ptr()).0)
            });
        // SAFETY: fetch_update closure always returns Some, so the result is alwyays Ok
        let result = unsafe { result.unwrap_unchecked() };
        // result is the previous value, so the read_state isn't set, only the read_ptr
        State(result).read_ptr()
    }

    pub(crate) fn release_read_lock(&self) {
        self.state
            .fetch_and(State::NOREAD_MASK, Ordering::Release);
    }

    /// tries to get the write lock to the ptr.
    pub(crate) fn lock_write(&self, ptr: Ptr) -> Result<(), ()> {
        let state = State(self.state.load(Ordering::Relaxed));
        if state.can_write(ptr) {
            // only need to synchronize with another thread when locking was successfull
            atomic::fence(Ordering::Acquire);
            Ok(())
        } else {
            Err(())
        }
    }

    /// Releases the read lock
    pub(crate) fn set_read_ptr(&self, ptr: Ptr) {
        match ptr {
            Ptr::Value1 => self.state.fetch_and(0b01111, Ordering::Release),
            Ptr::Value2 => self.state.fetch_or(0b10000, Ordering::Release),
        };
    }

    /// initializes the internal state. returns the ptr that
    pub(crate) fn initialize_state(this: &mut MaybeUninit<Self>) -> Ptr {
        // SAFETY: takes &mut self, so no writing is okay
        unsafe {
            (&raw mut (*this.as_mut_ptr()).state).write(AtomicU8::new(State::NOREAD_VALUE1_1ACCESS.0));
        }
        Ptr::Value2
    }

    pub(crate) fn get_value(&self, ptr: Ptr) -> &UnsafeCell<T> {
        match ptr {
            Ptr::Value1 => &self.value_1,
            Ptr::Value2 => &self.value_2,
        }
    }

    /// SAFETY: needs to have synchronized shared access to the ptr.
    /// If the access is not unique T needs to be Sync
    pub(crate) unsafe fn get_value_ref(&self, ptr: Ptr) -> &T {
        // SAFETY: requirements on the function make it safe
        unsafe { &*self.get_value(ptr).get() }
    }

    /// If self is unique increase the count and returns true.
    /// Otherwise returns false.
    ///
    /// If this returns true another smart pointer has to be created otherwise memory will be leaked
    pub(crate) unsafe fn is_unique_with_increase(&self) -> bool {
        if self.is_unique() {
            // relaxed taken from std Arc
            self.state.fetch_add(1, Ordering::Relaxed);
            true
        } else {
            false
        }
    }

    pub(crate) fn is_unique(&self) -> bool {
        let access = State(self.state.load(Ordering::Acquire)).access_count();
        access == 1
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
        let old_access = State(self.state.fetch_sub(1, Ordering::Release)).access_count();
        if old_access != 1 {
            return false;
        }
        // see std Arc
        atomic::fence(Ordering::Acquire);
        true
    }
}

/// SAFETY: same as SyncUnsafeCell. Synchronisation done by Reader and Writer
/// 
/// Isn't actually needed for the library as the public types have their own Send & Sync impls
/// which are needed as they have a ptr to Shared.
/// Clarifies that multithreaded refs are fine.
/// 
/// Send is autoimplemented, because UnsafeCell is Send if T: Send
unsafe impl<T: Sync> Sync for Shared<T> {}