use std::{num::NonZero, ops::ControlFlow};

use crate::{
    project::note_event::Note,
    sample::{ProcessingFrame, ProcessingFunction, Sample, SampleMetaData},
};

use super::Frame;

#[repr(u8)]
pub enum Interpolation {
    Nearest = 0,
    Linear = 1,
}

impl From<u8> for Interpolation {
    fn from(value: u8) -> Self {
        Self::from_u8(value)
    }
}

impl Interpolation {
    /// Amount of Padding in the SampleData to do each type of Interpolation.
    /// This much padding is needed at the start and end of the sample.
    pub const fn pad_needed(&self) -> usize {
        match self {
            Interpolation::Nearest => 1,
            Interpolation::Linear => 1,
        }
    }

    pub const fn from_u8(value: u8) -> Self {
        match value {
            0 => Self::Nearest,
            1 => Self::Linear,
            _ => panic!(),
        }
    }
}

#[derive(Debug)]
pub struct SamplePlayer {
    sample: Sample,
    meta: SampleMetaData,

    note: Note,
    // position in the sample, the next output frame should be.
    // Done this way, so 0 is a valid, useful and intuitive value
    // always a valid position in the sample. checked against sample lenght on each change
    // stored as fixed point data: usize + f32
    // f32 ranges 0..1
    position: (usize, f32),
    // is_done: bool,
    out_rate: NonZero<u32>,
    // how much the position is advanced for each output sample.
    // computed from in and out rate
    step_size: f32,
}

impl SamplePlayer {
    pub fn new(sample: Sample, meta: SampleMetaData, out_rate: NonZero<u32>, note: Note) -> Self {
        let step_size = Self::compute_step_size(meta.sample_rate, out_rate, meta.base_note, note);
        Self {
            sample,
            meta,
            position: (Sample::PAD_SIZE_EACH, 0.),
            out_rate,
            step_size,
            note,
        }
    }

    pub fn check_position(&self) -> ControlFlow<()> {
        if self.position.0 > self.sample.len_with_pad() - Sample::PAD_SIZE_EACH {
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(())
        }
    }

    #[inline]
    fn compute_step_size(
        in_rate: NonZero<u32>,
        out_rate: NonZero<u32>,
        sample_base_note: Note,
        playing_note: Note,
    ) -> f32 {
        // original formula: (outrate / inrate) * (playing_freq / sample_base_freq).
        // Where each freq is computed with MIDI tuning standard formula: 440 * 2^((note - 69)/12)
        // manually reduced formula: 2^((play_note - sample_base_note)/12) * (outrate / inrate)
        // herbie (https://herbie.uwplse.org/demo/index.html) can't optimize further: https://herbie.uwplse.org/demo/e096ef89ee257ad611dd56378bd139a065a6bea0.02e7ec5a3709ad3e06968daa97db50d636f1e44b/graph.html
        (f32::from(i16::from(playing_note.get()) - i16::from(sample_base_note.get())) / 12.).exp2()
            * (out_rate.get() as f32 / in_rate.get() as f32)
    }

    fn set_step_size(&mut self) {
        self.step_size = Self::compute_step_size(
            self.meta.sample_rate,
            self.out_rate,
            self.meta.base_note,
            self.note,
        );
    }

    pub fn set_out_samplerate(&mut self, samplerate: NonZero<u32>) {
        self.out_rate = samplerate;
        self.set_step_size();
    }

    /// steps self and sets is_done if needed
    fn step(&mut self) {
        self.position.1 += self.step_size;
        let floor = self.position.1.trunc();
        self.position.1 -= floor;
        self.position.0 += floor as usize;
    }

    pub fn iter<const INTERPOLATION: u8>(&mut self) -> SampleIter<'_, INTERPOLATION> {
        SampleIter { inner: self }
    }

    pub fn next<const INTERPOLATION: u8>(&mut self) -> Option<Frame> {
        // const block allows turning an invalid u8 into compile time error
        let interpolation = const { Interpolation::from_u8(INTERPOLATION) };

        if self.check_position().is_break() {
            return None;
        }

        let out = match interpolation {
            Interpolation::Nearest => self.compute_nearest(),
            Interpolation::Linear => self.compute_linear(),
        };

        self.step();
        Some(out)
    }

    fn compute_linear(&mut self) -> Frame {
        // There are two types that implement ProcessingFrame: f32 and Frame, so stereo and mono audio data.
        // the compiler will monomorphize this function to both versions and depending on wether that sample is mono
        // or stereo the correct version will be called.
        struct Linear(f32);
        impl<S: ProcessingFrame> ProcessingFunction<2, S> for Linear {
            fn process(self, data: &[S; 2]) -> S {
                let diff = data[1] - data[0];
                (diff * self.0) + data[0]
            }
        }
        self.sample
            .compute(self.position.0, Linear(self.position.1))
    }

    fn compute_nearest(&mut self) -> Frame {
        let load_idx = if self.position.1 < 0.5 {
            self.position.0
        } else {
            self.position.0 + 1
        };

        self.sample.index(load_idx)
    }
}

pub struct SampleIter<'player, const INTERPOLATION: u8> {
    inner: &'player mut SamplePlayer,
}

impl<const INTERPOLATION: u8> Iterator for SampleIter<'_, INTERPOLATION> {
    type Item = Frame;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next::<INTERPOLATION>()
    }
}
