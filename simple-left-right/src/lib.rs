//! Simpler version of the left-right from Jon Gjengset library.
//!
//! Uses two copies of the value to allow doing small changes, while still allowing non-blocking reading.
//! Writing can block, while reading doesn't.

#![warn(
    clippy::cargo,
    clippy::all,
    clippy::perf,
    clippy::style,
    clippy::complexity,
    clippy::suspicious,
    clippy::correctness,
    missing_docs,
    missing_copy_implementations,
    missing_debug_implementations,
    clippy::absolute_paths
)]
#![deny(
    unsafe_op_in_unsafe_fn,
    clippy::missing_safety_doc,
    clippy::undocumented_unsafe_blocks
)]
#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

#[cfg(feature = "std")]
use core::time::Duration;
#[cfg(feature = "std")]
use std::thread;

use core::{
    cell::UnsafeCell, marker::PhantomData, mem::MaybeUninit, ops::Deref, ptr::NonNull, sync::atomic::{fence, AtomicU8, Ordering}
};

use alloc::{collections::vec_deque::VecDeque, boxed::Box};

mod inner;

use inner::{Ptr, ReadState, Shared};

/// Should be implemented on structs that want to be shared with this library
pub trait Absorb<O> {
    /// has to be deterministic. Operations will be applied in the same order to both buffers
    fn absorb(&mut self, operation: O);
}

/// Dropping the Reader isn't realtime safe, because if dropped after the Writer, it deallocates.
/// Should only get dropped, when closing the real-time thread
///
/// Reader will be able to read data even if Writer has been dropped. Obviously that data won't change anymore
/// When there is no Reader the Writer is able to create a new one. The other way around doesn't work.
///
/// Isn't Sync as there is no methos that takes &self, so it is useless anyways.
#[derive(Debug)]
pub struct Reader<T> {
    shared: NonNull<Shared<T>>,
    /// for drop check
    _own: PhantomData<Shared<T>>,
}

impl<T> Reader<T> {
    const fn shared_ref(&self) -> &Shared<T> {
        // SAFETY: Reader always has a valid Shared<T>, a mut ref to a shared is never created,
        // only to the UnsafeCell<T>s inside of it
        unsafe { self.shared.as_ref() }
    }

    /// this function never blocks. (`fetch_update` loop doesn't count)
    pub fn lock(&mut self) -> ReadGuard<'_, T> {
        let inner_ref = self.shared_ref();
        // sets the corresponding read bit to the write ptr bit
        // happens as a single atomic operation so the 'double read' state isn't needed
        // ptr bit doesnt get changed
        // always Ok, as the passed closure never returns None
        let update_result =
            inner_ref
                .state
                .fetch_update(Ordering::Relaxed, Ordering::Acquire, |value| {
                    // SAFETY: At this point no Read bit is set, as creating a ReadGuard requires a &mut Reader and the Guard holds the &mut Reader
                    unsafe {
                        match Ptr::from_u8_no_read(value) {
                            Ptr::Value1 => Some(0b001),
                            Ptr::Value2 => Some(0b110),
                        }
                    }
                });

        // here the read ptr and read state of update_result match. maybe the atomic was already changed, but that doesn't matter.
        // we continue working with the state that we set.

        // SAFETY: the passed closure always returns Some, so fetch_update never returns Err
        let ptr = unsafe { Ptr::from_u8_no_read(update_result.unwrap_unchecked()) };

        // SAFETY: the Writer allowed the read on this value because the ptr bit was set. The read bit has been set
        let data = unsafe { inner_ref.get_value(ptr).get().as_ref().unwrap_unchecked() };

        ReadGuard {
            data,
            state: &inner_ref.state,
            reader: PhantomData,
        }
    }
}

/// SAFETY: Owns a T
unsafe impl<T: Send> Send for Reader<T> {}

impl<T> Drop for Reader<T> {
    fn drop(&mut self) {
        // SAFETY: Shared.should_drop() is called. on true object really is dropped. on false it isnt.
        // This is the last use of self and therefore also of Shared
        unsafe {
            let should_drop = self.shared_ref().should_drop();
            if should_drop {
                _ = Box::from_raw(self.shared.as_ptr());
            }
        }
    }
}

/// Data won't change while holding the Guard. This also means the Writer can only issue one swap, while Guard is being held
/// If T: !Sync this is guaranteed to be the only ref to this T
///
/// Doesn't implement Clone as that would require refcounting to know when to unlock.
#[derive(Debug)]
pub struct ReadGuard<'a, T> {
    data: &'a T,
    state: &'a AtomicU8,
    /// PhantomData makes the borrow checker prove that there only ever is one ReadGuard.
    /// This allows resetting the readstate without some kind of counter
    reader: PhantomData<&'a mut Reader<T>>,
}

impl<'a, T> Deref for ReadGuard<'a, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.data
    }
}

impl<T, E> AsRef<E> for ReadGuard<'_, T>
where
    E: ?Sized,
    T: AsRef<E>,
{
    fn as_ref(&self) -> &E {
        self.deref().as_ref()
    }
}

/// SAFETY: behaves like a ref to T. https://doc.rust-lang.org/std/marker/trait.Sync.html
unsafe impl<T: Sync> Send for ReadGuard<'_, T> {}
/// SAFETY: behaves like a ref to T. https://doc.rust-lang.org/std/marker/trait.Sync.html
unsafe impl<T: Sync> Sync for ReadGuard<'_, T> {}

impl<T> Drop for ReadGuard<'_, T> {
    fn drop(&mut self) {
        // release the read lock
        self.state.fetch_and(0b100, Ordering::Release);
    }
}

/// Not realtime safe object which can change the internal T value.
#[derive(Debug)]
pub struct Writer<T, O> {
    shared: NonNull<Shared<T>>,
    // sets which buffer the next write is applied to
    // write_ptr doesn't need to be Atomics as it only changes, when the Writer itself swaps
    write_ptr: Ptr,
    // buffer is pushed at the back and popped at the front.
    op_buffer: VecDeque<O>,
    // needed for drop_check
    _own: PhantomData<Shared<T>>,
}

impl<T, O> Writer<T, O> {
    const fn shared_ref(&self) -> &Shared<T> {
        // SAFETY: Reader always has a valid Shared<T>, a mut ref to a shared is never created,
        // only to the UnsafeCell<T>s inside of it
        unsafe { self.shared.as_ref() }
    }

    /// swaps the read and write values. If no changes were made since the last swap nothing happens. Never blocks
    /// not public as swapping without creating a before `WriteGuard` is pretty useless
    fn swap(&mut self) {
        if self.op_buffer.is_empty() {
            return;
        }

        match self.write_ptr {
            Ptr::Value1 => self.shared_ref().state.fetch_and(0b011, Ordering::Release),
            Ptr::Value2 => self.shared_ref().state.fetch_or(0b100, Ordering::Release),
        };

        self.write_ptr.switch();
    }

    /// get a Reader if none exists
    pub fn build_reader(&mut self) -> Option<Reader<T>> {
        // SAFETY: all is_unique_with_increase requirements are satisfied.
        unsafe {
            if self.shared_ref().is_unique_with_increase() {
                Some(Reader {
                    shared: self.shared,
                    _own: PhantomData,
                })
            } else {
                None
            }
        }
    }
}

impl<T: Absorb<O>, O> Writer<T, O> {
    /// Blocks if the Reader has a `ReadGuard` pointing to the old value.
    ///
    /// Uses a Spinlock because for anything else the OS needs to be involved and `Reader` can't talk to the OS.
    pub fn lock(&mut self) -> WriteGuard<'_, T, O> {
        let backoff = crossbeam_utils::Backoff::new();

        loop {
            // operation has to be aquire, but only the time it breaks the loop
            let state = self.shared_ref().state.load(Ordering::Relaxed);

            // SAFETY: is in state internal only value which is only set by library code
            let state = unsafe { ReadState::from_u8_ignore_ptr(state) };

            if state.can_write(self.write_ptr) {
                // make the load operation aquire only when it actually breaks the loop
                // the important (last) load is aquire, while all loads before are relaxed
                fence(Ordering::Acquire);
                break;
            }

            backoff.snooze();
        }

        // SAFETY: The spinloop before is only exited once the ReadState allows writing to the current
        // write_ptr value.
        unsafe { WriteGuard::new(self) }
    }

    /// Blocks if the Reader has a `ReadGuard` pointing to the old value.
    ///
    /// Uses a spin-lock, because the `Reader` can't talk to the OS. Sleeping and Yielding is done to avoid wasting cycles.
    /// Equivalent to ´lock´, except that it starts sleeping the given duration after a certaint point until the lock could be aquired.
    #[cfg(feature = "std")]
    pub fn sleep_lock(&mut self, sleep: Duration) -> WriteGuard<'_, T, O> {
        let backoff = crossbeam_utils::Backoff::new();

        loop {
            // operation has to be aquire, but only the time it breaks the loop
            let state = self.shared_ref().state.load(Ordering::Relaxed);

            // SAFETY: is in state internal only value which is only set by library code
            let state = unsafe { ReadState::from_u8_ignore_ptr(state) };

            if state.can_write(self.write_ptr) {
                // make the load operation aquire, only when it actually breaks the loop
                // the important (last) load is aquire, while all loads before are relaxed
                fence(Ordering::Acquire);
                break;
            }

            if backoff.is_completed() {
                thread::sleep(sleep);
            } else {
                backoff.snooze();
            }
        }

        // SAFETY: The spinloop before is only exited once the ReadState allows writing to the current
        // write_ptr value.
        unsafe { WriteGuard::new(self) }
    }

    /// Equivalent to `lock` but the sleeping is done asyncly to not block the runtime.
    /// It is still a spinlock, it just give control to the runtime when locking is slow, before trying again.
    #[cfg(feature = "async")]
    pub async fn async_lock(&mut self, sleep: Duration) -> WriteGuard<'_, T, O> {
        let backoff = crossbeam_utils::Backoff::new();

        loop {
            // operation has to be aquire, but only the time it breaks the loop
            let state = self.shared_ref().state.load(Ordering::Relaxed);

            // SAFETY: is in state internal only value which is only set by library code
            let state = unsafe { ReadState::from_u8_ignore_ptr(state) };

            if state.can_write(self.write_ptr) {
                // make the load operation aquire, only when it actually breaks the loop
                // the important (last) load is aquire, while all loads before are relaxed
                fence(Ordering::Acquire);
                break;
            }

            if backoff.is_completed() {
                async_io::Timer::after(sleep).await;
            } else {
                backoff.spin();
                futures_lite::future::yield_now().await;
            }
        }

        // SAFETY: The spinloop before is only exited once the ReadState allows writing to the current
        // write_ptr value.
        unsafe { WriteGuard::new(self) }
    }

    /// doesn't block. Returns None if the Reader has a `ReadGuard` pointing to the old value
    pub fn try_lock(&mut self) -> Option<WriteGuard<'_, T, O>> {
        let state = self.shared_ref().state.load(Ordering::Acquire);

        // SAFETY: is in state internal only value which is only set by library code
        let state = unsafe { ReadState::from_u8_ignore_ptr(state) };

        if state.can_write(self.write_ptr) {
            // SAFETY: ReadState allows this
            unsafe { Some(WriteGuard::new(self)) }
        } else {
            None
        }
    }
}

impl<T: Clone, O> Writer<T, O> {
    /// Creates a new Writer by cloning the value once to get two values
    pub fn new(value: T) -> Self {
        // SAFETY: ptr was just alloced, so is valid and unique.
        let mut shared: Box<MaybeUninit<Shared<T>>> = Box::new_uninit();
        Shared::initialize_state(&mut shared);
        let shared_ptr = shared.as_mut_ptr();

        // SAFETY: Every field gets initialized, ptr is valid and doesn't alias
        let shared = unsafe {
            UnsafeCell::raw_get(&raw const (*shared_ptr).value_1).write(value.clone());
            UnsafeCell::raw_get(&raw const (*shared_ptr).value_2).write(value);
            // consumes the Box<MaybeUninit> and creates the NonNull with an initialized value
            NonNull::new_unchecked(Box::into_raw(shared.assume_init()))
        };

        Writer {
            shared,
            write_ptr: Ptr::Value2,
            op_buffer: VecDeque::new(),
            _own: PhantomData,
        }
    }
}

impl<T: Default, O> Default for Writer<T, O> {
    /// Creates a new Writer by calling `T::default()` twice to create the two values
    ///
    /// Default impl of T needs to give the same result every time. Not upholding this doens't lead to UB, but turns the library basically useless
    ///
    /// Could leak a T object if T::default() panics.
    fn default() -> Self {
        // SAFETY: ptr was just alloced, so is valid and unique.
        let mut shared: Box<MaybeUninit<Shared<T>>> = Box::new_uninit();
        Shared::initialize_state(&mut shared);
        let shared_ptr = shared.as_mut_ptr();

        // SAFETY: Every field gets initialized, ptr is valid and doesn't alias
        let shared = unsafe {
            UnsafeCell::raw_get(&raw const (*shared_ptr).value_1).write(T::default());
            UnsafeCell::raw_get(&raw const (*shared_ptr).value_2).write(T::default());
            // consumes the Box<MaybeUninit> and creates the NonNull with an initialized value
            NonNull::new_unchecked(Box::into_raw(shared.assume_init()))
        };

        Writer {
            shared,
            write_ptr: Ptr::Value2,
            op_buffer: VecDeque::new(),
            _own: PhantomData,
        }
    }
}

impl<T: Sync, O> Writer<T, O> {
    /// The Value returned may be newer than the version the reader is currently seeing.
    /// This value will be written to next.
    ///
    /// Needs T: Sync because maybe this is the value the reader is curently reading
    pub fn read(&self) -> &T {
        // SAFETY: Only the WriteGuard can write to the values / create mut refs to them.
        // The WriteGuard holds a mut ref to the writer so this function can't be called while a writeguard exists
        // This means that reading them / creating refs is safe to do
        unsafe {
            self.shared_ref()
                .get_value(self.write_ptr)
                .get()
                .as_ref()
                .unwrap_unchecked()
        }
    }
}

/// SAFETY: owns T and O
unsafe impl<T: Send, O: Send> Send for Writer<T, O> {}
/// SAFETY: &self fn can only create a &T and doesn't allow access to O
unsafe impl<T: Sync, O> Sync for Writer<T, O> {}

impl<T, O> Drop for Writer<T, O> {
    fn drop(&mut self) {
        // SAFETY: Shared.should_drop() is called. on true object really is dropped. on false it isnt.
        // This is the last use of self and therefore also of Shared
        unsafe {
            let should_drop = self.shared_ref().should_drop();
            if should_drop {
                _ = Box::from_raw(self.shared.as_ptr());
            }
        }
    }
}

// Don't create a WriteGuard directly, as that wouldn't sync with old Operations
/// Can be used to write to the Data structure.
///
/// When this structure exists the Reader already switched to the other value
///
/// Dropping this makes all changes available to the Reader.
#[derive(Debug)]
pub struct WriteGuard<'a, T, O> {
    writer: &'a mut Writer<T, O>,
}

impl<T, O> WriteGuard<'_, T, O> {
    /// Makes the changes available to the reader. Equivalent to std::mem::drop(self)
    pub fn swap(self) {}

    /// Gets the value currently being written to.
    pub fn read(&self) -> &T {
        // SAFETY: Only the WriteGuard can write to the values / create mut refs to them.
        // The WriteGuard holds a mut ref to the writer so this function can't be called while a writeguard exists
        // This means that reading them / creating refs is safe to do
        unsafe {
            self.writer.shared_ref()
                .get_value(self.writer.write_ptr)
                .get()
                .as_ref()
                .unwrap_unchecked()
        }
    }

    /// Isn't public as this could easily create disconnects between the two versions.
    /// While that wouldn't lead to UB it goes against the purpose of this library
    fn get_data_mut(&mut self) -> &mut T {
        // SAFETY: When creating the writeguad it is checked that the reader doesnt have access to the same data
        // This function requires &mut self so there also isn't any ref created by writeguard.
        // SAFETY: the ptr is never null, therefore unwrap_unchecked
        unsafe {
            self.writer
                .shared_ref()
                .get_value(self.writer.write_ptr)
                .get()
                .as_mut()
                .unwrap_unchecked()
        }
    }
}

impl<'a, T: Absorb<O>, O> WriteGuard<'a, T, O> {
    /// created a new `WriteGuard` and syncs the two values if needed.
    ///
    /// ### SAFETY
    /// No `ReadGuard` is allowed to exist to the same value the `Writer.write_ptr` points to
    ///
    /// Assuming a correct `Reader` & `ReadGuard` implementation:
    /// If Inner.read_state.can_write(Writer.write_ptr) == true this function is fine to call
    unsafe fn new(writer: &'a mut Writer<T, O>) -> Self {
        let mut guard = Self { writer };
        while let Some(operation) = guard.writer.op_buffer.pop_front() {
            guard.get_data_mut().absorb(operation);
        }
        guard
    }
}

impl<T: Absorb<O>, O: Clone> WriteGuard<'_, T, O> {
    /// applies operation to the current write Value and stores it to apply to the other later.
    /// If there is no reader the operation is applied to both values immediately and not stored.
    pub fn apply_op(&mut self, operation: O) {
        if self.writer.shared_ref().is_unique() {
            let shared_ref = self.writer.shared_ref();
            // SAFETY: is_unique checked that no Reader exists. I am the only one with access to Shared<T>, so i can modify whatever i want.
            unsafe {
                (*shared_ref.value_1.get()).absorb(operation.clone());
                (*shared_ref.value_2.get()).absorb(operation);
            }
        } else {
            self.writer.op_buffer.push_back(operation.clone());
            self.get_data_mut().absorb(operation);
        }
    }
}

/// SAFETY: behaves like a &mut T and &mut Vec<O>. https://doc.rust-lang.org/stable/std/marker/trait.Sync.html
unsafe impl<T: Send, O: Send> Send for WriteGuard<'_, T, O> {}

/// Safety: can only create shared refs to T, not to O. https://doc.rust-lang.org/stable/std/marker/trait.Sync.html
unsafe impl<T: Sync, O> Sync for WriteGuard<'_, T, O> {}

impl<T, O> Drop for WriteGuard<'_, T, O> {
    fn drop(&mut self) {
        self.writer.swap();
    }
}
