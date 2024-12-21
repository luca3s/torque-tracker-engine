use std::{array, fmt::Debug, mem::ManuallyDrop, ops::Deref, sync::Arc};

use dasp::sample::FromSample;

use crate::{
    audio_processing::Frame, file::impulse_format::sample::VibratoWave, manager::Collector,
    project::note_event::Note,
};

pub(crate) union SampleHandle<'a, const GC: bool> {
    gc: ManuallyDrop<SharedSample>,
    reference: ManuallyDrop<SampleRef<'a>>,
}

impl<const GC: bool> Debug for SampleHandle<'_, GC> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match GC {
            true => f
                .debug_struct("GC Sample Handle")
                .field("data", unsafe { &self.gc })
                .finish(),
            false => f
                .debug_struct("Ref Sample")
                .field("data", unsafe { &self.reference })
                .finish(),
        }
    }
}

impl<const GC: bool> Clone for SampleHandle<'_, GC> {
    fn clone(&self) -> Self {
        match GC {
            true => Self {
                gc: unsafe { self.gc.clone() },
            },
            false => Self {
                reference: unsafe { self.reference },
            },
        }
    }
}

impl<const GC: bool> SampleHandle<'_, GC> {
    pub fn get_ref(&self) -> SampleRef<'_> {
        match GC {
            true => unsafe { self.gc.deref().borrow() },
            false => unsafe { *self.reference.deref() },
        }
    }
}

impl<const GC: bool> Drop for SampleHandle<'_, GC> {
    fn drop(&mut self) {
        // references don't need to be dropped
        if GC {
            unsafe { ManuallyDrop::drop(&mut self.gc) }
        }
    }
}

pub union Sample<const GC: bool> {
    gc: ManuallyDrop<SharedSample>,
    owned: ManuallyDrop<OwnedSample>,
}

impl Sample<true> {
    pub(crate) fn new(value: SharedSample) -> Self {
        Self {
            gc: ManuallyDrop::new(value),
        }
    }

    pub(crate) fn get_handle(&self) -> SampleHandle<'static, true> {
        let data = unsafe { self.gc.clone() };
        SampleHandle { gc: data }
    }

    pub(crate) fn take(mut self) -> SharedSample {
        let out = unsafe { ManuallyDrop::take(&mut self.gc) };
        std::mem::forget(self);
        out
    }

    pub(crate) fn from_owned(mut owned: Sample<false>, handle: &mut Collector) -> Self {
        let data = unsafe { ManuallyDrop::take(&mut owned.owned) };
        std::mem::forget(owned);
        Sample {
            gc: ManuallyDrop::new(handle.add_sample(data)),
        }
    }
}

impl Sample<false> {
    pub fn new(value: OwnedSample) -> Self {
        Self {
            owned: ManuallyDrop::new(value),
        }
    }

    pub(crate) fn get_handle(&self) -> SampleHandle<'_, false> {
        let data = unsafe { self.owned.deref().borrow() };
        SampleHandle {
            reference: ManuallyDrop::new(data),
        }
    }

    pub fn take(mut self) -> OwnedSample {
        let out = unsafe { ManuallyDrop::take(&mut self.owned) };
        std::mem::forget(self);
        out
    }
}

impl<const GC: bool> Sample<GC> {
    pub(crate) fn get_ref(&self) -> SampleRef<'_> {
        match GC {
            true => unsafe { self.gc.deref().borrow() },
            false => unsafe { self.owned.deref().borrow() },
        }
    }
}

impl<const GC: bool> Clone for Sample<GC> {
    fn clone(&self) -> Self {
        match GC {
            true => Self {
                gc: unsafe { self.gc.clone() },
            },
            false => Self {
                owned: unsafe { self.owned.clone() },
            },
        }
    }
}

impl<const GC: bool> Debug for Sample<GC> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match GC {
            true => f
                .debug_struct("Shared Sample")
                .field("data", unsafe { &self.gc })
                .finish(),
            false => f
                .debug_struct("Boxed Sample")
                .field("data", unsafe { &self.owned })
                .finish(),
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

#[derive(Debug, Clone, Copy)]
pub(crate) enum SampleRef<'a> {
    MonoF32(&'a [f32]),
    MonoI16(&'a [i16]),
    MonoI8(&'a [i8]),
    StereoF32(&'a [[f32; 2]]),
    StereoI16(&'a [[i16; 2]]),
    StereoI8(&'a [[i8; 2]]),
}

impl SampleRef<'_> {
    pub fn index(&self, index: usize) -> Frame {
        match self {
            SampleRef::MonoF32(d) => d[index].into(),
            SampleRef::MonoI16(d) => d[index].into(),
            SampleRef::MonoI8(d) => d[index].into(),
            SampleRef::StereoF32(d) => d[index].into(),
            SampleRef::StereoI16(d) => d[index].into(),
            SampleRef::StereoI8(d) => d[index].into(),
        }
    }

    pub fn index_stereo(&self, index: usize) -> Frame {
        match self {
            SampleRef::StereoF32(d) => d[index].into(),
            SampleRef::StereoI16(d) => d[index].into(),
            SampleRef::StereoI8(d) => d[index].into(),
            _ => unreachable!(),
        }
    }

    /// index..index + N
    pub fn index_stereo_array<const N: usize>(&self, index: usize) -> [Frame; N] {
        match self {
            SampleRef::StereoF32(d) => array::from_fn(|i| d[index + i].into()),
            SampleRef::StereoI16(d) => array::from_fn(|i| d[index + i].into()),
            SampleRef::StereoI8(d) => array::from_fn(|i| d[index + i].into()),
            _ => unreachable!(),
        }
    }

    pub fn index_mono_array<const N: usize>(&self, index: usize) -> [f32; N] {
        match self {
            SampleRef::MonoF32(d) => array::from_fn(|i| d[index + i]),
            SampleRef::MonoI16(d) => array::from_fn(|i| f32::from_sample_(d[index + i])),
            SampleRef::MonoI8(d) => array::from_fn(|i| f32::from_sample_(d[index + i])),
            _ => unreachable!(),
        }
    }

    // index..index+N
    /// calls process once or twice depending on stereo or mono
    pub fn compute<const N: usize, F: Fn([f32; N]) -> f32>(
        &self,
        process: F,
        index: usize,
    ) -> Frame {
        if self.is_mono() {
            let arr = self.index_mono_array(index);
            Frame::from(process(arr))
        } else {
            let arr = self.index_stereo_array(index);
            let (left, right) = Frame::split_array(arr);
            Frame::from([process(left), process(right)])
        }
    }

    pub fn index_mono(&self, index: usize) -> f32 {
        match self {
            SampleRef::MonoF32(d) => d[index],
            SampleRef::MonoI16(d) => d[index].into(),
            SampleRef::MonoI8(d) => d[index].into(),
            _ => unreachable!(),
        }
    }

    pub fn len_with_pad(&self) -> usize {
        match self {
            SampleRef::MonoF32(d) => d.len(),
            SampleRef::MonoI16(d) => d.len(),
            SampleRef::MonoI8(d) => d.len(),
            SampleRef::StereoF32(d) => d.len(),
            SampleRef::StereoI16(d) => d.len(),
            SampleRef::StereoI8(d) => d.len(),
        }
    }

    pub fn is_mono(&self) -> bool {
        match self {
            SampleRef::MonoF32(_) => true,
            SampleRef::MonoI16(_) => true,
            SampleRef::MonoI8(_) => true,
            SampleRef::StereoF32(_) => false,
            SampleRef::StereoI16(_) => false,
            SampleRef::StereoI8(_) => false,
        }
    }
}

#[derive(Debug, Clone)]
pub enum OwnedSample {
    MonoF32(Box<[f32]>),
    MonoI16(Box<[i16]>),
    MonoI8(Box<[i8]>),
    StereoF32(Box<[[f32; 2]]>),
    StereoI16(Box<[[i16; 2]]>),
    StereoI8(Box<[[i8; 2]]>),
}

impl OwnedSample {
    pub(crate) fn borrow(&self) -> SampleRef<'_> {
        match self {
            OwnedSample::MonoF32(d) => SampleRef::MonoF32(d),
            OwnedSample::MonoI16(d) => SampleRef::MonoI16(d),
            OwnedSample::MonoI8(d) => SampleRef::MonoI8(d),
            OwnedSample::StereoF32(d) => SampleRef::StereoF32(d),
            OwnedSample::StereoI16(d) => SampleRef::StereoI16(d),
            OwnedSample::StereoI8(d) => SampleRef::StereoI8(d),
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) enum SharedSample {
    MonoF32(Arc<[f32]>),
    MonoI16(Arc<[i16]>),
    MonoI8(Arc<[i8]>),
    StereoF32(Arc<[[f32; 2]]>),
    StereoI16(Arc<[[i16; 2]]>),
    StereoI8(Arc<[[i8; 2]]>),
}

impl SharedSample {
    pub fn borrow(&self) -> SampleRef<'_> {
        match self {
            SharedSample::MonoF32(d) => SampleRef::MonoF32(d),
            SharedSample::MonoI16(d) => SampleRef::MonoI16(d),
            SharedSample::MonoI8(d) => SampleRef::MonoI8(d),
            SharedSample::StereoF32(d) => SampleRef::StereoF32(d),
            SharedSample::StereoI16(d) => SampleRef::StereoI16(d),
            SharedSample::StereoI8(d) => SampleRef::StereoI8(d),
        }
    }
}

pub const MAX_LENGTH: usize = 16000000;
pub const MAX_RATE: usize = 192000;
/// this many frames need to be put on the start and the end
pub const PAD_SIZE_EACH: usize = 4;

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
