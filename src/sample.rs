use std::{
    fmt::Debug,
    iter::repeat_n,
    ops::{Add, AddAssign, Mul, MulAssign, Sub, SubAssign},
    sync::Arc,
};

use crate::{
    audio_processing::Frame, file::impulse_format::sample::VibratoWave, project::note_event::Note,
};

pub(crate) trait ProcessingFrame:
    Add<Self, Output = Self>
    + AddAssign<Self>
    + Sub<Self, Output = Self>
    + SubAssign<Self>
    + Mul<f32, Output = Self>
    + MulAssign<f32>
    + Copy
{
}

impl ProcessingFrame for Frame {}

impl ProcessingFrame for f32 {}

pub(crate) trait ProcessingFunction<const N: usize, Fr: ProcessingFrame> {
    fn process(self, data: &[Fr; N]) -> Fr;
}

#[derive(Clone)]
pub struct Sample {
    mono: bool,
    data: Arc<[f32]>,
}

impl Sample {
    pub const MAX_LENGTH: usize = 16_000_000;
    pub const MAX_RATE: usize = 192_000;
    /// this many frames need to be put on the start and the end to ensure that the interpolation algorithms work correctly.
    pub const PAD_SIZE_EACH: usize = 4;

    pub fn is_mono(&self) -> bool {
        self.mono
    }

    /// len in Frames
    pub fn len_with_pad(&self) -> usize {
        if self.mono {
            self.data.len()
        } else {
            self.data.len() / 2
        }
    }

    pub(crate) fn compute<
        const N: usize,
        Proc: ProcessingFunction<N, f32> + ProcessingFunction<N, Frame>,
    >(
        &self,
        index: usize,
        proc: Proc,
    ) -> Frame {
        if self.is_mono() {
            let data: &[f32; N] = self.data[index..index + N].try_into().unwrap();
            Frame::from(proc.process(data))
        } else {
            let data: &[Frame; N] = Frame::from_interleaved(&self.data[index * 2..(index + N) * 2])
                .try_into()
                .unwrap();
            proc.process(data)
        }
    }

    pub fn index(&self, idx: usize) -> Frame {
        if self.is_mono() {
            Frame::from(self.data[idx])
        } else {
            Frame::from([self.data[idx * 2], self.data[idx * 2 + 1]])
        }
    }

    pub(crate) fn strongcount(&self) -> usize {
        Arc::strong_count(&self.data)
    }

    pub fn new_stereo_interpolated<I: IntoIterator<Item = f32>>(data: I) -> Self {
        Self::new_stereo_interpolated_padded(
            repeat_n(0f32, 2 * Self::PAD_SIZE_EACH)
                .chain(data)
                .chain(repeat_n(0f32, 2 * Self::PAD_SIZE_EACH)),
        )
    }

    pub fn new_stereo_interpolated_padded<I: IntoIterator<Item = f32>>(data: I) -> Self {
        Self {
            mono: false,
            data: Arc::from_iter(data),
        }
    }

    pub fn new_mono<I: IntoIterator<Item = f32>>(data: I) -> Self {
        Self::new_mono_padded(
            repeat_n(0f32, Self::PAD_SIZE_EACH)
                .chain(data)
                .chain(repeat_n(0f32, Self::PAD_SIZE_EACH)),
        )
    }

    pub fn new_mono_padded<I: IntoIterator<Item = f32>>(data: I) -> Self {
        Self {
            mono: true,
            data: Arc::from_iter(data),
        }
    }
}

impl Debug for Sample {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Sample")
            .field("mono", &self.mono)
            .field("data_len", &self.len_with_pad())
            .finish_non_exhaustive()
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
