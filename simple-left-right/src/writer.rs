use crate::{reader::Reader, Ptr, ReadState, Shared};
use core::{
    cell::UnsafeCell,
    sync::atomic::{AtomicU8, Ordering, fence},
};
use std::{borrow::Borrow, collections::VecDeque, ops::Deref, sync::Arc};

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
            self.get_data_mut().absorb(operation.clone());
            self.writer.op_buffer.push_back(operation);
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
    <Self as Deref>::Target: AsRef<E>,
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

impl<'a, T: Absorb<O>, O: Clone> Writer<T, O> {
    /// blocks if the Reader has a `ReadGuard` pointing to the old value
    pub fn lock(&'a mut self) -> WriteGuard<'a, T, O> {
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
    pub fn try_lock(&'a mut self) -> Option<WriteGuard<'a, T, O>> {
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
        // allow value 1 to be read
        let inner = Shared {
            value_1: UnsafeCell::new(value.clone()),
            value_2: UnsafeCell::new(value),
            state: AtomicU8::new(0b000), // read from 0, no reads currently
        };
        // set value 2 to be written to
        Writer {
            shared: Arc::new(inner),
            write_ptr: Ptr::Value2,
            op_buffer: VecDeque::new(),
            just_swapped: false,
        }
    }

    // WAIT FOR ARC::new_uninit to be stabilized. probably on september 5th

    // pub fn new_from_box(value: Box<T>) -> Self {
    //     let shared = {
    //         let value_1 = unsafe { transmute::<Box<T>, Box<ManuallyDrop<T>>>(value) };
    //         let value_2 = value_1.clone();

    //         let mut uninit = Arc::new(MaybeUninit::uninit());
    //         let mut_ref = Arc::get_mut(&mut uninit).unwrap();
    //         // build the UnsafeCells
    //         unsafe { addr_of_mut!((*mut_ref.as_mut_ptr()).value_1).write(UnsafeCell::new(MaybeUninit::uninit())) };

    //         // unsafe { addr_of_mut!((*mut_ref.as_mut_ptr()).value_1).write(UnsafeCell::new(T::default())) };
    //         // unsafe { addr_of_mut!((*mut_ref.as_mut_ptr()).value_2).write(UnsafeCell::new(T::default())) };
    //         unsafe { addr_of_mut!((*mut_ref.as_mut_ptr()).state).write(AtomicU32::new(0b000)) };

    //         // assume init
    //         unsafe { transmute::<Arc<MaybeUninit<Shared<T>>>, Arc<Shared<T>>>(uninit) }
    //     };
    //     Writer {
    //         shared,
    //         write_ptr: Ptr::Value2,
    //         op_buffer: VecDeque::new(),
    //         just_swapped: false,
    //     }
    // }
}

impl<T: Default, O> Default for Writer<T, O> {
    fn default() -> Self {
        let shared = Shared {
            value_1: UnsafeCell::new(T::default()),
            value_2: UnsafeCell::new(T::default()),
            state: AtomicU8::new(0b000),
        };
        Writer {
            shared: Arc::new(shared),
            write_ptr: Ptr::Value2,
            op_buffer: VecDeque::new(),
            just_swapped: false,
        }
    }

    //     /// default impl needs to give the same result every time
    //     /// if default panics memory will probably be leaked
    //     pub fn new_from_default() -> Self {
    //         let shared = {
    //             let mut uninit: Arc<MaybeUninit<Shared<T>>> = Arc::new(MaybeUninit::uninit());
    //             // get mut ref to arc because only one arc exists
    //             let mut_ref = Arc::get_mut(&mut uninit).unwrap();

    //             // initialize everything
    //             unsafe { addr_of_mut!((*mut_ref.as_mut_ptr()).value_1).write(UnsafeCell::new(T::default())) };
    //             unsafe { addr_of_mut!((*mut_ref.as_mut_ptr()).value_2).write(UnsafeCell::new(T::default())) };
    //             unsafe { addr_of_mut!((*mut_ref.as_mut_ptr()).state).write(AtomicU32::new(0b000)) };

    //             // assume init
    //             unsafe { transmute::<Arc<MaybeUninit<Shared<T>>>, Arc<Shared<T>>>(uninit) }
    //         };
    //         Writer {
    //             shared,
    //             write_ptr: Ptr::Value2,
    //             op_buffer: VecDeque::new(),
    //             just_swapped: false,
    //         }
    //     }
}

impl<T, O> Borrow<T> for Writer<T, O> {
    fn borrow(&self) -> &T {
        self
    }
}

impl<T, O, E> AsRef<E> for Writer<T, O>
where
    E: ?Sized,
    <Self as Deref>::Target: AsRef<E>,
{
    fn as_ref(&self) -> &E {
        self.deref().as_ref()
    }
}

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

pub trait Absorb<O> {
    /// has to be deterministic. Operations will be applied in the same order to both buffers
    fn absorb(&mut self, operation: O);
}
