use std::{borrow::Borrow, fmt::Debug, mem::ManuallyDrop, ops::Deref};

use basedrop::Shared;

use crate::{file::impulse_format::sample::VibratoWave, project::note_event::Note};

// This ugliness won't be needed anymore as soon as Return Type Notatiion in type positions is available
// https://blog.rust-lang.org/inside-rust/2024/09/26/rtn-call-for-testing.html
// I could also keep it as it simplifies API everywhere else a lot and makes sure only my defined Types can "impl the trait"
pub(crate) union SampleRef<'a, const GC: bool> {
    gc: ManuallyDrop<Shared<SampleData>>,
    reference: &'a SampleData,
}

impl SampleRef<'static, true> {
    pub(crate) fn new(data: Shared<SampleData>) -> Self {
        SampleRef {
            gc: ManuallyDrop::new(data),
        }
    }

    pub fn get(&self) -> &Shared<SampleData> {
        unsafe { &self.gc }
    }
}

impl<'a> SampleRef<'a, false> {
    pub fn new(data: &'a SampleData) -> Self {
        SampleRef { reference: data }
    }

    pub fn get(&self) -> &SampleData {
        unsafe { self.reference }
    }
}

impl<const GC: bool> Debug for SampleRef<'_, GC> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match GC {
            true => write!(f, "GC Sample Ref"),
            false => write!(f, "Non GC Sample Ref"),
        }
    }
}

impl<const GC: bool> Deref for SampleRef<'_, GC> {
    type Target = SampleData;

    fn deref(&self) -> &Self::Target {
        match GC {
            true => unsafe { self.gc.deref() },
            false => unsafe { self.reference },
        }
    }
}

impl<const GC: bool> Drop for SampleRef<'_, GC> {
    fn drop(&mut self) {
        if GC {
            unsafe { ManuallyDrop::drop(&mut self.gc) };
        }
    }
}

impl<const GC: bool> Clone for SampleRef<'_, GC> {
    fn clone(&self) -> Self {
        match GC {
            true => {
                let data = unsafe { self.gc.deref().clone() };
                SampleRef {
                    gc: ManuallyDrop::new(data),
                }
            }
            false => {
                let data = unsafe { self.reference };
                SampleRef { reference: data }
            }
        }
    }
}

pub union Sample<const GC: bool> {
    gc: ManuallyDrop<Shared<SampleData>>,
    owned: ManuallyDrop<SampleData>,
}

impl Sample<true> {
    pub fn new(data: Shared<SampleData>) -> Self {
        Self {
            gc: ManuallyDrop::new(data),
        }
    }

    pub(crate) fn get_ref(&self) -> SampleRef<'static, true> {
        let data = unsafe { self.gc.deref().clone() };
        SampleRef::<'static, true>::new(data)
    }

    pub fn get(&self) -> &Shared<SampleData> {
        unsafe { &self.gc }
    }

    pub fn take(mut self) -> Shared<SampleData> {
        let out = unsafe { ManuallyDrop::take(&mut self.gc) };
        std::mem::forget(self);
        out
    }

    pub fn to_owned(&self) -> Sample<false> {
        let shared = unsafe { self.gc.deref() }.deref();
        Sample::<false>::new(shared.clone())
    }
}

impl Sample<false> {
    pub fn new(data: SampleData) -> Self {
        Self {
            owned: ManuallyDrop::new(data),
        }
    }

    pub fn get(&self) -> &SampleData {
        unsafe { &self.owned }
    }

    pub(crate) fn get_ref<'a>(&'a self) -> SampleRef<'a, false> {
        SampleRef::<'a, false>::new(unsafe { &self.owned })
    }

    pub fn take(mut self) -> SampleData {
        let out = unsafe { ManuallyDrop::take(&mut self.owned) };
        std::mem::forget(self);
        out
    }

    // avoids copying the underlaying data. As soon as SampleData gets ?Sized this should change
    #[expect(clippy::wrong_self_convention)]
    pub(crate) fn to_gc(self, handle: &basedrop::Handle) -> Sample<true> {
        let data = self.take();
        let shared = basedrop::Shared::new(handle, data);
        Sample::<true>::new(shared)
    }
}

impl<const GC: bool> Debug for Sample<GC> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match GC {
            true => write!(f, "GC Sample. len: {}", self.borrow().len_with_pad()),
            false => write!(f, "Owned Sample. len: {}", self.borrow().len_with_pad()),
        }
    }
}

impl<const GC: bool> Deref for Sample<GC> {
    type Target = SampleData;

    fn deref(&self) -> &Self::Target {
        match GC {
            true => unsafe { self.gc.deref() },
            false => unsafe { self.owned.deref() },
        }
    }
}

impl<const GC: bool> Clone for Sample<GC> {
    fn clone(&self) -> Self {
        match GC {
            true => {
                let data = unsafe { self.gc.deref().clone() };
                Sample {
                    gc: ManuallyDrop::new(data),
                }
            }
            false => {
                let data = unsafe { self.owned.deref().clone() };
                Sample {
                    owned: ManuallyDrop::new(data),
                }
            }
        }
    }
}

impl<const GC: bool> Drop for Sample<GC> {
    fn drop(&mut self) {
        match GC {
            true => unsafe { ManuallyDrop::drop(&mut self.gc) },
            false => unsafe { ManuallyDrop::drop(&mut self.owned) },
        }
    }
}

/// samples need to be padded with PAD_SIZE frames at the start and end
#[derive(Clone, Debug)]
pub enum SampleData {
    Mono(Box<[f32]>),
    Stereo(Box<[[f32; 2]]>),
}

impl SampleData {
    pub const MAX_LENGTH: usize = 16000000;
    pub const MAX_RATE: usize = 192000;
    /// this many frames need to be put on the start and the end
    pub const PAD_SIZE_EACH: usize = 4;

    pub fn len_with_pad(&self) -> usize {
        match self {
            SampleData::Mono(m) => m.len(),
            SampleData::Stereo(s) => s.len(),
        }
    }
}

// mono impl
impl FromIterator<f32> for SampleData {
    fn from_iter<T: IntoIterator<Item = f32>>(iter: T) -> Self {
        let iter = iter.into_iter();
        let size_hint = iter.size_hint();
        let mut data = if let Some(upper_bound) = size_hint.1 {
            Vec::with_capacity(upper_bound + (Self::PAD_SIZE_EACH * 2))
        } else {
            Vec::with_capacity(size_hint.0 + (Self::PAD_SIZE_EACH * 2))
        };

        data.extend_from_slice(&[0.; Self::PAD_SIZE_EACH]);
        data.extend(iter);
        data.extend_from_slice(&[0.; Self::PAD_SIZE_EACH]);

        Self::Mono(data.into_boxed_slice())
    }
}

// stereo impl
impl FromIterator<[f32; 2]> for SampleData {
    fn from_iter<T: IntoIterator<Item = [f32; 2]>>(iter: T) -> Self {
        let iter = iter.into_iter();
        let size_hint = iter.size_hint();
        let mut data = if let Some(upper_bound) = size_hint.1 {
            Vec::with_capacity(upper_bound + (Self::PAD_SIZE_EACH * 2))
        } else {
            Vec::with_capacity(size_hint.0 + (Self::PAD_SIZE_EACH * 2))
        };

        data.extend_from_slice(&[[0.; 2]; Self::PAD_SIZE_EACH]);
        data.extend(iter);
        data.extend_from_slice(&[[0.; 2]; Self::PAD_SIZE_EACH]);

        Self::Stereo(data.into_boxed_slice())
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SampleMetaData {
    pub default_volume: u8,
    pub global_volume: u8,
    pub default_pan: Option<u8>,
    pub vibrato_speed: u8,
    pub vibrato_depth: u8,
    pub vibrato_rate: u8,
    pub vibrato_waveform: VibratoWave,
    pub sample_rate: u32,
    pub base_note: Note,
}
