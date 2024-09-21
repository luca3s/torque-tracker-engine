#![warn(
    clippy::all,
    clippy::pedantic,
    clippy::perf,
    clippy::style,
    clippy::complexity,
    clippy::suspicious,
    clippy::correctness
)]

use std::{
    borrow::Borrow,
    cell::UnsafeCell,
    collections::VecDeque,
    marker::PhantomData,
    ops::Deref,
    sync::{
        atomic::{fence, AtomicU8, Ordering},
        Arc,
    },
};

mod inner;

use inner::{Ptr, ReadState, Shared};

pub trait Absorb<O> {
    /// has to be deterministic. Operations will be applied in the same order to both buffers
    fn absorb(&mut self, operation: O);
}

/// Data won't change while holding the Guard. This also means the Writer can only issue one swap, while Guard is being held
#[derive(Debug)]
pub struct ReadGuard<'a, T> {
    data: &'a UnsafeCell<T>,
    state: &'a AtomicU8,
    // PhantomData makes the borrow checker prove that there only ever is one ReadGuard
    // This is needed because on Drop the ReadGuard sets current_read to None
    reader: PhantomData<&'a mut Reader<T>>,
}

impl<'a, T> Deref for ReadGuard<'a, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        unsafe { &*self.data.get() }
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

impl<T> Drop for ReadGuard<'_, T> {
    fn drop(&mut self) {
        // release the read lock
        self.state.fetch_and(0b100, Ordering::Release);
    }
}

/// Dropping the Reader isn't realtime safe, because if dropped after the Writer, it deallocates.
/// Should only get dropped, when closing the real-time thread
///
/// Reader will be able to read data even if Writer has been dropped. Obviously that data won't change anymore
/// When there is no Reader the Writer is able to create a new one. The other way around doesn't work
pub struct Reader<T> {
    inner: Arc<Shared<T>>,
}

impl<T> Reader<T> {
    /// this function never blocks
    pub fn lock(&mut self) -> ReadGuard<'_, T> {
        // sets the corresponding read bit to the write ptr bit
        // happens as a single atomic operation so the 'double read' state isn't needed
        // ptr bit doesnt get changed
        let ptr = unsafe {
            self.inner
                .state
                .fetch_update(Ordering::Relaxed, Ordering::Acquire, |value| {
                    match value.into() {
                        Ptr::Value1 => Some(0b001),
                        Ptr::Value2 => Some(0b110),
                    }
                })
                .unwrap_unchecked()
                .into()
        };

        let data = self.inner.get_value(ptr);

        ReadGuard {
            data,
            state: &self.inner.state,
            reader: PhantomData,
        }
    }
}

pub struct WriteGuard<'a, T, O> {
    writer: &'a mut Writer<T, O>,
}

impl<T, O> WriteGuard<'_, T, O> {
    /// see swap on Writer.
    /// drops `WriteGuard`, because the creation of a new `WriteGuard` has to wait for the Reader to drop his `ReadGuard`.
    pub fn swap(self) {
        self.writer.swap();
    }

    fn get_data_mut(&mut self) -> &mut T {
        // SAFETY: When creating the writeguad it is checked that the reader doesnt have access to the same data
        // This function requires &mut self so there also isn't any ref created by writeguard.
        unsafe {
            self.writer
                .shared
                .get_value(self.writer.write_ptr)
                .get()
                .as_mut()
                .unwrap()
        }
    }
}

impl<'a, T: Absorb<O>, O> WriteGuard<'a, T, O> {
    /// syncs the two values with the operation Buffer
    fn new_after_swap(writer: &'a mut Writer<T, O>) -> Self {
        writer.just_swapped = false;
        let mut guard = Self { writer };
        while let Some(operation) = guard.writer.op_buffer.pop_front() {
            guard.get_data_mut().absorb(operation);
        }
        guard
    }
}

impl<T: Absorb<O>, O: Clone> WriteGuard<'_, T, O> {
    /// applies operation to the current write Value and stores it to apply to the other later.
    /// If there is no reader the operation is applied to both values immediately and not stored
    pub fn apply_op(&mut self, operation: O) {
        if let Some(inner) = Arc::get_mut(&mut self.writer.shared) {
            inner.value_1.get_mut().absorb(operation.clone());
            inner.value_2.get_mut().absorb(operation);
        } else {
            self.writer.op_buffer.push_back(operation.clone());
            self.get_data_mut().absorb(operation);
        }
    }
}

impl<T, O> Borrow<T> for WriteGuard<'_, T, O> {
    fn borrow(&self) -> &T {
        self
    }
}

impl<T, O, E> AsRef<E> for WriteGuard<'_, T, O>
where
    E: ?Sized,
    T: AsRef<E>,
{
    fn as_ref(&self) -> &E {
        self.deref().as_ref()
    }
}

impl<T, O> Deref for WriteGuard<'_, T, O> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &*self.writer
    }
}

pub struct Writer<T, O> {
    shared: Arc<Shared<T>>,
    // sets which buffer the next write is applied to
    // write_ptr doesn't need to be Atomics as it only changes, when the Writer itself swaps
    write_ptr: Ptr,
    // buffer is pushed at the back and popped at the front.
    op_buffer: VecDeque<O>,
    just_swapped: bool,
}

impl<T, O> Writer<T, O> {
    /// swaps the read and write values. If no changes were made since the last swap nothing happens. Never blocks
    /// see also `WriteGuard::swap`, which is maybe a bit more ergonimic
    pub fn swap(&mut self) {
        if self.op_buffer.is_empty() {
            return;
        }

        match self.write_ptr {
            Ptr::Value1 => self.shared.state.fetch_and(0b011, Ordering::Release),
            Ptr::Value2 => self.shared.state.fetch_or(0b100, Ordering::Release),
        };

        self.write_ptr.switch();
        self.just_swapped = true;
    }

    /// get a Reader if none exists
    pub fn build_reader(&mut self) -> Option<Reader<T>> {
        if Arc::get_mut(&mut self.shared).is_some() {
            Some(Reader {
                inner: self.shared.clone(),
            })
        } else {
            None
        }
    }
}

impl<T: Absorb<O>, O> Writer<T, O> {
    /// blocks if the Reader has a `ReadGuard` pointing to the old value
    pub fn lock(&mut self) -> WriteGuard<'_, T, O> {
        let backoff = crossbeam_utils::Backoff::new();

        loop {
            // operation has to be aquire, but only the time it breaks the loop
            let state = self.shared.state.load(Ordering::Relaxed);

            if ReadState::from(state).can_write(self.write_ptr) {
                // make the load operation aquire only when it actually breaks the loop
                // the important (last) load is aquire, while all loads before are relaxed
                fence(Ordering::Acquire);
                break;
            }

            backoff.snooze();
        }

        if self.just_swapped {
            WriteGuard::new_after_swap(self)
        } else {
            WriteGuard { writer: self }
        }
    }

    /// doesn't block. Returns None if the Reader has a `ReadGuard` pointing to the old value
    pub fn try_lock(&mut self) -> Option<WriteGuard<'_, T, O>> {
        let state = self.shared.state.load(Ordering::Acquire);

        if ReadState::from(state).can_write(self.write_ptr) {
            if self.just_swapped {
                Some(WriteGuard::new_after_swap(self))
            } else {
                Some(WriteGuard { writer: self })
            }
        } else {
            None
        }
    }
}

impl<T: Clone, O> Writer<T, O> {
    pub fn new(value: T) -> Self {
        let mut shared: Arc<std::mem::MaybeUninit<Shared<T>>> = Arc::new_uninit();
        let shared_ptr: *mut Shared<T> =
            unsafe { Arc::get_mut(&mut shared).unwrap_unchecked() }.as_mut_ptr();

        let state_ptr: *mut AtomicU8 = unsafe { &raw mut (*shared_ptr).state };
        unsafe { state_ptr.write(AtomicU8::new(0b000)) };

        let value_1_ptr: *mut UnsafeCell<T> = unsafe { &raw mut (*shared_ptr).value_1 };
        // SAFETY: UnsafeCell<T> has the same memory Layout as T
        unsafe { (value_1_ptr as *mut T).write(value.clone()) };

        let value_2_ptr: *mut UnsafeCell<T> = unsafe { &raw mut (*shared_ptr).value_2 };
        // SAFETY: UnsafeCell<T> has the same memory Layout as T
        unsafe { (value_2_ptr as *mut T).write(value) };

        // SAFETY: all fields of shared were initialized
        let shared: Arc<Shared<T>> = unsafe { shared.assume_init() };
        Writer {
            shared,
            write_ptr: Ptr::Value2,
            op_buffer: VecDeque::new(),
            just_swapped: false,
        }
    }
}

impl<T: Default, O> Default for Writer<T, O> {
    /// Default impl of T needs to give the same result every time
    fn default() -> Self {
        let mut shared: Arc<std::mem::MaybeUninit<Shared<T>>> = Arc::new_uninit();
        let shared_ptr: *mut Shared<T> =
            unsafe { Arc::get_mut(&mut shared).unwrap_unchecked() }.as_mut_ptr();

        let state_ptr: *mut AtomicU8 = unsafe { &raw mut (*shared_ptr).state };
        unsafe { state_ptr.write(AtomicU8::new(0b000)) };

        let value_1_ptr: *mut UnsafeCell<T> = unsafe { &raw mut (*shared_ptr).value_1 };
        // SAFETY: UnsafeCell<T> has the same memory Layout as T
        unsafe { (value_1_ptr as *mut T).write(T::default()) };

        let value_2_ptr: *mut UnsafeCell<T> = unsafe { &raw mut (*shared_ptr).value_2 };
        // SAFETY: UnsafeCell<T> has the same memory Layout as T
        unsafe { (value_2_ptr as *mut T).write(T::default()) };

        // SAFETY: all fields of shared were initialized
        let shared: Arc<Shared<T>> = unsafe { shared.assume_init() };
        Writer {
            shared,
            write_ptr: Ptr::Value2,
            op_buffer: VecDeque::new(),
            just_swapped: false,
        }
    }
}

impl<T, O> Borrow<T> for Writer<T, O> {
    fn borrow(&self) -> &T {
        self
    }
}

impl<T, O, E> AsRef<E> for Writer<T, O>
where
    E: ?Sized,
    T: AsRef<E>,
{
    fn as_ref(&self) -> &E {
        self.deref().as_ref()
    }
}

/// This impl is only ok because this is an internal library.
/// If one would publish the library it would lead to function name collisions
/// If there ever is a internal collision, rename a function or remove this impl
impl<T, O> Deref for Writer<T, O> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        // SAFETY: Only the WriteGuard can write to the values / create mut refs to them.
        // The WriteGuard holds a mut ref to the writer so this function can't be called while a writeguard exists
        // This means that reading them / creating refs is safe to do
        unsafe {
            self.shared
                .get_value(self.write_ptr)
                .get()
                .as_ref()
                .unwrap()
        }
    }
}
