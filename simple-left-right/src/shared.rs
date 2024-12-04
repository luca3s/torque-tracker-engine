use core::{
    cell::UnsafeCell,
    mem::MaybeUninit,
    sync::atomic::{self, AtomicU8, Ordering},
};

#[derive(Clone, Copy, Debug)]
#[repr(u8)]
pub(crate) enum Ptr {
    Value1 = 0,
    Value2 = 0b1000,
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
#[derive(Debug, Clone, Copy)]
struct State(u8);

impl State {
    /// - Read Ptr: Value 1,
    /// - No Read
    /// - Unique
    const INITIAL: u8 = 0b0000;

    const VALUE1_READ: u8 = 0b0010;
    const VALUE2_READ: u8 = 0b0100;
    const READ_MASK: u8 = 0b0110;
    const NOREAD_MASK: u8 = 0b1001;
    const UNIQUE_MASK: u8 = 0b0001;
    const INV_UNIQUE_MASK: u8 = 0b1110;
    const READ_PTR_MASK: u8 = 0b1000;
    const INV_READ_PTR: u8 = 0b0111;

    #[inline]
    // only does debug tests that state is valid
    // could be turned into assert_uncheckeds as they are still checkd in debug
    fn new(value: u8) -> Self {
        // max 1 read
        debug_assert!((value & Self::READ_MASK).count_ones() <= 1, "{value:b}");
        // only lower 4 bits are used
        debug_assert!(value & 0b11110000 == 0, "{value:b}");
        Self(value)
    }

    fn is_unique(self) -> bool {
        self.0 & Self::UNIQUE_MASK == 0
    }

    fn read_ptr(self) -> Ptr {
        // mask out everything except the read ptr
        if self.0 & Self::READ_PTR_MASK == 0 {
            Ptr::Value1
        } else {
            Ptr::Value2
        }
    }

    fn with_read(self, ptr: Ptr) -> Self {
        // no read state exists.
        debug_assert_eq!(self.0 & Self::READ_MASK, 0);
        let mask = match ptr {
            Ptr::Value1 => Self::VALUE1_READ,
            Ptr::Value2 => Self::VALUE2_READ,
        };

        Self(self.0 | mask)
    }

    fn can_write(self, ptr: Ptr) -> bool {
        #[expect(clippy::match_like_matches_macro)] // i think it's more readable like this
        match (self.0 & Self::READ_MASK, ptr) {
            (Self::VALUE1_READ, Ptr::Value1) => false,
            (Self::VALUE2_READ, Ptr::Value2) => false,
            _ => true,
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
    /// | 0 | 0: unique, 1: second object exists |
    /// | 1 | is value 1 being read |
    /// | 2 | is value 2 being read |
    /// | 3 | which value should be read next (0: value 1, 1: value 2) |
    ///
    /// This mixed use doesn't lead to more contention because there are only two threads max.
    state: AtomicU8,
}

impl<T> Shared<T> {
    pub(crate) fn lock_read(&self) -> Ptr {
        // fetch update loop could be replaced with:
        // - set read state to both
        // - read read ptr
        // - set read state to only that
        // this would need to be synchronized correctly and is probably not faster than this
        let result = self
            .state
            .fetch_update(Ordering::Relaxed, Ordering::Acquire, |value| {
                let state = State::new(value);
                let ptr = state.read_ptr();
                Some(state.with_read(ptr).0)
            });
        // SAFETY: fetch_update closure always returns Some, so the result is alwyays Ok
        let result = unsafe { result.unwrap_unchecked() };
        // result is the previous value, so the read_state isn't set, only the read_ptr
        State::new(result).read_ptr()
    }

    pub(crate) fn release_read_lock(&self) {
        self.state.fetch_and(State::NOREAD_MASK, Ordering::Release);
    }

    /// tries to get the write lock to the ptr.
    pub(crate) fn lock_write(&self, ptr: Ptr) -> Result<(), ()> {
        let state = State::new(self.state.load(Ordering::Relaxed));
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
        let value = match ptr {
            Ptr::Value1 => self.state.fetch_and(State::INV_READ_PTR, Ordering::Release),
            Ptr::Value2 => self.state.fetch_or(State::READ_PTR_MASK, Ordering::Release),
        };
        State::new(value);
    }

    /// initializes the internal state. returns the ptr that
    pub(crate) fn initialize_state(this: &mut MaybeUninit<Self>) -> Ptr {
        // SAFETY: takes &mut self, so writing is okay
        unsafe {
            (&raw mut (*this.as_mut_ptr()).state).write(AtomicU8::new(State::INITIAL));
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
    pub(crate) unsafe fn set_shared(&self) {
        self.state.fetch_or(State::UNIQUE_MASK, Ordering::Relaxed);
    }

    pub(crate) fn is_unique(&self) -> bool {
        State::new(self.state.load(Ordering::Acquire)).is_unique()
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
        if !State::new(
            self.state
                .fetch_and(State::INV_UNIQUE_MASK, Ordering::Release),
        )
        .is_unique()
        {
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
