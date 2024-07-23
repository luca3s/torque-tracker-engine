use std::{
    marker::PhantomData, ops::{AddAssign, ControlFlow, Deref}, path::MAIN_SEPARATOR
};

use basedrop::Shared;

use crate::sample::{SampleData, SampleMetaData};

use super::Frame;

#[repr(u8)]
pub enum Interpolation {
    Nearest = 0,
    Linear = 1,
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

pub struct SamplePlayer {
    sample: Shared<SampleData>,
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

impl SamplePlayer {
    pub fn new(sample: (SampleMetaData, Shared<SampleData>), out_rate: u32, in_rate: u32) -> Self {
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
        if self.position.0 > self.sample.len_with_pad() - SampleData::PAD_SIZE_EACH {
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(())
        }
    }

    fn compute_step_size(in_rate: u32, out_rate: u32) -> f32 {
        out_rate as f32 / in_rate as f32
    }

    fn step(&mut self) {
        self.position.1 += self.step_size;
        let floor = self.position.1.floor();
        self.position.1 -= floor;
        self.position.0 += floor as usize;
    }

    pub fn iter<const T: u8>(&mut self, volume: f32) -> SampleIter<T> {
        SampleIter { sample_player: self, volume }
    }
}

/// https://github.com/rust-lang/rust/issues/95174
/// feature(adt_const_params)
pub(crate) struct SampleIter<'a, const INTERPOLATION: u8> {
    sample_player: &'a mut SamplePlayer,
    volume: f32,
}

impl Iterator for SampleIter<'_, { Interpolation::Nearest as u8 }> {
    type Item = Frame;

    fn next(&mut self) -> Option<Self::Item> {
        if self.sample_player.check_position().is_break() {
            return None;
        }

        let load_idx = if self.sample_player.position.1 < 0.5 {
            self.sample_player.position.0
        } else {
            self.sample_player.position.0 + 1
        };

        let mut out = match self.sample_player.sample.deref() {
            SampleData::Mono(mono) => mono[load_idx].into(),
            SampleData::Stereo(stereo) => stereo[load_idx].into(),
        };

        out *= self.volume;
        self.sample_player.step();
        Some(out)
    }
}

impl Iterator for SampleIter<'_, { Interpolation::Linear as u8 }> {
    type Item = Frame;

    fn next(&mut self) -> Option<Self::Item> {
        if self.sample_player.check_position().is_break() {
            return None;
        }

        let mut out = match self.sample_player.sample.deref() {
            SampleData::Mono(mono) => {
                let diff =
                    mono[self.sample_player.position.0 + 1] - mono[self.sample_player.position.0];
                Frame::from(
                    (diff * self.sample_player.position.1) + mono[self.sample_player.position.0],
                )
            }
            SampleData::Stereo(stereo) => {
                let diff: Frame = Frame::from(stereo[self.sample_player.position.0 + 1])
                    - Frame::from(stereo[self.sample_player.position.0]);

                (diff * self.sample_player.position.1)
                    + stereo[self.sample_player.position.0].into()
            }
        };

        out *= self.volume;
        self.sample_player.step();
        Some(out)
    }
}
