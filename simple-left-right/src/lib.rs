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
    data: &'a T,
    state: &'a AtomicU8,
    // PhantomData makes the borrow checker prove that there only ever is one ReadGuard
    //
    // This is needed because setting the ReadState can only be reset when no ReadGuard exists
    // and that would mean some kind of counter
    reader: PhantomData<&'a mut Reader<T>>,
}

// only struct that should have this impl, as it doesn't have any methods
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
    /// this function never blocks. (fetch_update loop doesn't count)
    pub fn lock(&mut self) -> ReadGuard<'_, T> {
        // sets the corresponding read bit to the write ptr bit
        // happens as a single atomic operation so the 'double read' state isn't needed
        // ptr bit doesnt get changed
        let update_result =
            self.inner
                .state
                .fetch_update(Ordering::Relaxed, Ordering::Acquire, |value| {
                    // SAFETY: At this point no Read bit is set, as creating a ReadGuard requires a &mut Reader and the Guard holds the &mut Reader
                    unsafe {
                        std::hint::assert_unchecked(value & 0b011 == 0);
                    }
                    match value.into() {
                        Ptr::Value1 => Some(0b001),
                        Ptr::Value2 => Some(0b110),
                    }
                });

        // SAFETY: the passed clorusure always returns Some, so fetch_update never returns Err
        let ptr = unsafe { update_result.unwrap_unchecked().into() };

        // SAFETY: the Writer always sets the Read bit to the opposite of its write_ptr
        let data = unsafe { self.inner.get_value(ptr).get().as_ref().unwrap_unchecked() };

        // SAFETY: the read_state is set to the value that is being
        ReadGuard {
            data,
            state: &self.inner.state,
            reader: PhantomData,
        }
    }
}

// Don't ever create a WriteGuard directly
/// Can be used to write to the Data structure.
///
/// When this structure exists the Reader already switched to the other value
/// 
/// Dropping this makes all changes available to the Reader
pub struct WriteGuard<'a, T, O> {
    writer: &'a mut Writer<T, O>,
}

impl<T, O> WriteGuard<'_, T, O> {
    /// Makes the changes available to the reader.
    pub fn swap(self) {}

    /// Gets the value currently being written to.
    pub fn read(&self) -> &T {
        self.writer.read()
    }

    /// Isn't public as this could easily create disconnects between the two versions.
    /// While that wouldn't lead to UB it goes against the purpose of this library
    fn get_data_mut(&mut self) -> &mut T {
        // SAFETY: When creating the writeguad it is checked that the reader doesnt have access to the same data
        // This function requires &mut self so there also isn't any ref created by writeguard.
        unsafe { self.get_data_ptr().as_mut().unwrap() }
    }

    fn get_data_ptr(&self) -> *mut T {
        self.writer.shared.get_value(self.writer.write_ptr).get()
    }
}

impl<'a, T: Absorb<O>, O> WriteGuard<'a, T, O> {
    /// created a new WriteGuard and syncs the two values if needed.
    ///
    /// ### SAFETY
    /// No ReadGuard is allowed to exist to the same value as Writer.write_ptr points to
    /// 
    /// Assuming a correct Reader & ReadGuard implementation:
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
        if let Some(inner) = Arc::get_mut(&mut self.writer.shared) {
            inner.value_1.get_mut().absorb(operation.clone());
            inner.value_2.get_mut().absorb(operation);
        } else {
            self.writer.op_buffer.push_back(operation.clone());
            self.get_data_mut().absorb(operation);
        }
    }
}

impl<T, O> Drop for WriteGuard<'_, T, O> {
    fn drop(&mut self) {
        self.writer.swap();
    }
}

pub struct Writer<T, O> {
    shared: Arc<Shared<T>>,
    // sets which buffer the next write is applied to
    // write_ptr doesn't need to be Atomics as it only changes, when the Writer itself swaps
    write_ptr: Ptr,
    // buffer is pushed at the back and popped at the front.
    op_buffer: VecDeque<O>,
}

impl<T, O> Writer<T, O> {
    /// swaps the read and write values. If no changes were made since the last swap nothing happens. Never blocks
    /// not public as swapping without creating a WriteGuard is pretty
    fn swap(&mut self) {
        if self.op_buffer.is_empty() {
            return;
        }

        match self.write_ptr {
            Ptr::Value1 => self.shared.state.fetch_and(0b011, Ordering::Release),
            Ptr::Value2 => self.shared.state.fetch_or(0b100, Ordering::Release),
        };

        self.write_ptr.switch();
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

    // Gets the value that will be written to next
    pub fn read(&self) -> &T {
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

impl<T: Absorb<O>, O> Writer<T, O> {
    /// Blocks if the Reader has a `ReadGuard` pointing to the old value.
    ///
    /// Uses a Spinlock because for anything else the OS needs to be involved. Reader can't talk to the OS.
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

        // SAFETY: The spinloop before is only exited once the ReadState allows writing to the current
        // write_ptr value.
        unsafe { WriteGuard::new(self) }
    }

    /// doesn't block. Returns None if the Reader has a `ReadGuard` pointing to the old value
    pub fn try_lock(&mut self) -> Option<WriteGuard<'_, T, O>> {
        let state = self.shared.state.load(Ordering::Acquire);

        if ReadState::from(state).can_write(self.write_ptr) {
            // SAFETY: ReadState allows this
            unsafe { Some(WriteGuard::new(self)) }
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
        }
    }
}