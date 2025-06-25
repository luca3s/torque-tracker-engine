use std::{
    fmt::Debug,
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
    pub fn is_mono(&self) -> bool {
        self.mono
    }

    pub fn len_with_pad(&self) -> usize {
        if self.mono {
            self.data.len()
        } else {
            self.data.len() / 2
        }
    }

    pub fn compute<
        const N: usize,
        Proc: ProcessingFunction<N, f32> + ProcessingFunction<N, Frame>,
    >(
        &self,
        index: usize,
        proc: Proc,
    ) -> Frame {
        if self.is_mono() {
            let data: &[Frame; N] = Frame::from_interleaved(&self.data[index * 2..(index + N) * 2])
                .try_into()
                .unwrap();
            proc.process(data)
        } else {
            let data: &[f32; N] = self.data[index..index + N].try_into().unwrap();
            Frame::from(proc.process(data))
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
}

impl Debug for Sample {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Sample")
            .field("mono", &self.mono)
            .field("data_len", &self.len_with_pad())
            .finish_non_exhaustive()
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
