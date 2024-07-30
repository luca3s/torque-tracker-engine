use std::ops::{ControlFlow, Deref};

use crate::sample::{SampleData, SampleMetaData};

use super::Frame;

#[repr(u8)]
pub enum Interpolation {
    Nearest = 0,
    Linear = 1,
}

impl From<u8> for Interpolation {
    fn from(value: u8) -> Self {
        match value {
            0 => Self::Nearest,
            1 => Self::Linear,
            _ => panic!(),
        }
    }
}

impl Interpolation {
    /// Amount of Padding in the SampleData to do each type of Interpolation.
    /// This much padding is needed at the start and end of the sample.
    pub const fn pad_needed(&self) -> usize {
        match self {
            Interpolation::Nearest => 1,
            Interpolation::Linear => todo!(),
        }
    }
}

// 'a is the lifetime of the sampleData
pub struct SamplePlayer<S: Deref<Target = SampleData>> {
    sample: S,
    meta_data: SampleMetaData,
    // position in the sample, the next output frame should be.
    // Done this way, so 0 is a valid, useful and intuitive value
    // always a valid position in the sample. checked against sample lenght on each change
    // stored as fixed point data: usize + f32
    // f32 ranges 0..1
    position: (usize, f32),

    out_rate: u32,
    in_rate: u32,
    // how much the position is advanced for each output sample.
    // computed from in and out rate
    step_size: f32,
}

impl<S: Deref<Target = SampleData>> SamplePlayer<S> {
    pub fn new(sample: (SampleMetaData, S), out_rate: u32, in_rate: u32) -> Self {
        Self {
            sample: sample.1,
            meta_data: sample.0,
            position: (SampleData::PAD_SIZE_EACH, 0.),
            out_rate,
            in_rate,
            step_size: Self::compute_step_size(in_rate, out_rate),
        }
    }

    pub fn check_position(&self) -> ControlFlow<()> {
        if self.position.0 > self.sample.deref().len_with_pad() - SampleData::PAD_SIZE_EACH {
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(())
        }
    }

    fn compute_step_size(in_rate: u32, out_rate: u32) -> f32 {
        out_rate as f32 / in_rate as f32
    }

    pub fn set_out_samplerate(&mut self, samplerate: u32) {
        self.out_rate = samplerate;
        self.step_size = Self::compute_step_size(self.in_rate, samplerate)
    }

    fn step(&mut self) {
        self.position.1 += self.step_size;
        let floor = self.position.1.floor();
        self.position.1 -= floor;
        self.position.0 += floor as usize;
    }

    pub fn iter<const INTERPOLATION: u8>(&mut self) -> SampleIter<INTERPOLATION, S> {
        SampleIter {
            sample_player: self,
        }
    }

    pub fn next<const INTERPOLATION: u8>(&mut self) -> Option<Frame> {
        match Interpolation::from(INTERPOLATION) {
            Interpolation::Nearest => self.next_nearest(),
            Interpolation::Linear => self.next_linear(),
        }
    }

    pub fn next_linear(&mut self) -> Option<Frame> {
        if self.check_position().is_break() {
            return None;
        }

        let out = match self.sample.deref() {
            SampleData::Mono(mono) => {
                let diff = mono[self.position.0 + 1] - mono[self.position.0];
                Frame::from((diff * self.position.1) + mono[self.position.0])
            }
            SampleData::Stereo(stereo) => {
                let diff: Frame =
                    Frame::from(stereo[self.position.0 + 1]) - Frame::from(stereo[self.position.0]);

                (diff * self.position.1) + stereo[self.position.0].into()
            }
        };

        self.step();
        Some(out)
    }

    pub fn next_nearest(&mut self) -> Option<Frame> {
        if self.check_position().is_break() {
            return None;
        }

        let load_idx = if self.position.1 < 0.5 {
            self.position.0
        } else {
            self.position.0 + 1
        };

        let out = match self.sample.deref() {
            SampleData::Mono(mono) => mono[load_idx].into(),
            SampleData::Stereo(stereo) => stereo[load_idx].into(),
        };

        self.step();
        Some(out)
    }
}

/// https://github.com/rust-lang/rust/issues/95174
/// feature(adt_const_params)
// a: lifetime of the samples, b: lifetime of the SamplePlayer
pub(crate) struct SampleIter<'b, const INTERPOLATION: u8, S: Deref<Target = SampleData>> {
    sample_player: &'b mut SamplePlayer<S>,
}

impl<const INTERPOLATION: u8, S: Deref<Target = SampleData>> Iterator
    for SampleIter<'_, INTERPOLATION, S>
{
    type Item = Frame;

    fn next(&mut self) -> Option<Self::Item> {
        self.sample_player.next::<INTERPOLATION>()
    }
}
