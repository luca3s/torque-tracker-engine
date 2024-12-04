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
#![no_std]

extern crate alloc;

use core::{cell::UnsafeCell, marker::PhantomData, mem::MaybeUninit, ops::Deref, ptr::NonNull};

use alloc::{boxed::Box, collections::vec_deque::VecDeque};

mod shared;

use shared::{Ptr, Shared};

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
        let shared_ref = self.shared_ref();

        ReadGuard {
            shared: shared_ref,
            value: shared_ref.lock_read(),
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
    shared: &'a Shared<T>,
    value: Ptr,
    /// PhantomData makes the borrow checker prove that there only ever is one ReadGuard.
    /// This allows resetting the readstate without some kind of counter
    reader: PhantomData<&'a mut Reader<T>>,
}

impl<'a, T> Deref for ReadGuard<'a, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        // SAFETY: ReadGuard was created, so the Writer knows not to write in this spot
        unsafe { self.shared.get_value_ref(self.value) }
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

// /// SAFETY: behaves like a ref to T. https://doc.rust-lang.org/std/marker/trait.Sync.html
// unsafe impl<T: Sync> Send for ReadGuard<'_, T> {}
// /// SAFETY: behaves like a ref to T. https://doc.rust-lang.org/std/marker/trait.Sync.html
// unsafe impl<T: Sync> Sync for ReadGuard<'_, T> {}

impl<T> Drop for ReadGuard<'_, T> {
    fn drop(&mut self) {
        // release the read lock
        self.shared.release_read_lock();
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
        // SAFETY: Reader always has a valid Shared<T>, the only possibility to get a &mut Shared requires &mut self
        unsafe { self.shared.as_ref() }
    }

    /// if no Reader exists this gives a mut ref to Shared.
    fn shared_mut(&mut self) -> Option<&mut Shared<T>> {
        if self.shared_ref().is_unique() {
            // SAFETY: No Reader exists, as is_unique returns true
            Some(unsafe { &mut *self.shared.as_ptr() })
        } else {
            None
        }
    }

    /// swaps the read and write values. If no changes were made since the last swap nothing happens. Never blocks
    /// not public as swapping without creating a before `WriteGuard` is pretty useless
    fn swap(&mut self) {
        if self.op_buffer.is_empty() {
            return;
        }

        self.shared_ref().set_read_ptr(self.write_ptr);

        self.write_ptr.switch();
    }

    /// get a Reader if none exists
    pub fn build_reader(&mut self) -> Option<Reader<T>> {
        let shared_ref = self.shared_ref();
        // SAFETY: all is_unique_with_increase requirements are satisfied.
        unsafe {
            if shared_ref.is_unique() {
                shared_ref.set_shared();
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
    /// doesn't block. Returns None if the Reader has a `ReadGuard` pointing to the old value.
    #[must_use]
    pub fn try_lock(&mut self) -> Option<WriteGuard<'_, T, O>> {
        self.shared_ref()
            .lock_write(self.write_ptr)
            .ok()
            // SAFETY: locking was successful
            .map(|_| unsafe { WriteGuard::new(self) })
    }
}

impl<T: Clone, O> Writer<T, O> {
    /// Creates a new Writer by cloning the value once to get two values
    pub fn new(value: T) -> Self {
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
        unsafe { self.shared_ref().get_value_ref(self.write_ptr) }
    }
}

/// SAFETY: owns T and O
unsafe impl<T: Send, O: Send> Send for Writer<T, O> {}
/// SAFETY: &self fn can only create a &T and never gives shared access to O
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
            self.writer
                .shared_ref()
                .get_value_ref(self.writer.write_ptr)
        }
    }

    /// Isn't public as this could easily create disconnects between the two versions.
    /// While that wouldn't lead to UB it goes against the purpose of this library
    fn get_data_mut(&mut self) -> &mut T {
        // SAFETY: When creating the writeguad it is checked that the reader doesnt have access to the same data
        // This function requires &mut self so there also isn't any ref created by writeguard.
        unsafe {
            &mut *self
                .writer
                .shared_ref()
                .get_value(self.writer.write_ptr)
                .get()
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
        if let Some(shared) = self.writer.shared_mut() {
            shared.value_1.get_mut().absorb(operation.clone());
            shared.value_2.get_mut().absorb(operation);
        } else {
            self.writer.op_buffer.push_back(operation.clone());
            self.get_data_mut().absorb(operation);
        }
    }
}

// /// SAFETY: behaves like a &mut T and &mut Vec<O>. https://doc.rust-lang.org/stable/std/marker/trait.Sync.html
// unsafe impl<T: Send, O: Send> Send for WriteGuard<'_, T, O> {}

// /// Safety: can only create shared refs to T, not to O. https://doc.rust-lang.org/stable/std/marker/trait.Sync.html
// unsafe impl<T: Sync, O> Sync for WriteGuard<'_, T, O> {}

impl<T, O> Drop for WriteGuard<'_, T, O> {
    fn drop(&mut self) {
        self.writer.swap();
    }
}

#[cfg(test)]
mod internal_test {
    use core::cell::Cell;

    use crate::{Absorb, Writer};

    #[derive(Clone, Copy, Debug)]
    pub struct CounterAddOp(i32);

    impl Absorb<CounterAddOp> for i32 {
        fn absorb(&mut self, operation: CounterAddOp) {
            *self += operation.0;
        }
    }

    impl Absorb<CounterAddOp> for Cell<i32> {
        fn absorb(&mut self, operation: CounterAddOp) {
            self.set(self.get() + operation.0);
        }
    }

    #[test]
    fn drop_reader() {
        let mut writer: Writer<i32, CounterAddOp> = Writer::default();
        let reader = writer.build_reader().unwrap();

        assert!(!writer.shared_ref().is_unique());
        drop(reader);
        assert!(writer.shared_ref().is_unique());
    }

    #[test]
    fn drop_writer() {
        let mut writer: Writer<i32, CounterAddOp> = Writer::default();
        let reader = writer.build_reader().unwrap();

        assert!(!reader.shared_ref().is_unique());
        drop(writer);
        assert!(reader.shared_ref().is_unique());
    }
}
