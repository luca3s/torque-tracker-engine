use std::ops::{ControlFlow, Deref};

use crate::{
    sample::{SampleData, SampleMetaData, SampleRef},
    song::note_event::Note,
};

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

#[derive(Debug)]
enum RealTimeEffectState {
    PitchSlide
}

#[derive(Debug)]
pub struct SamplePlayer<'sample, const GC: bool> {
    sample: SampleRef<'sample, GC>,
    meta_data: SampleMetaData,

    note: Note,
    // position in the sample, the next output frame should be.
    // Done this way, so 0 is a valid, useful and intuitive value
    // always a valid position in the sample. checked against sample lenght on each change
    // stored as fixed point data: usize + f32
    // f32 ranges 0..1
    position: (usize, f32),

    out_rate: u32,
    // how much the position is advanced for each output sample.
    // computed from in and out rate
    step_size: f32,
}

impl<'sample, const GC: bool> SamplePlayer<'sample, GC> {
    pub fn new(
        sample: (SampleMetaData, SampleRef<'sample, GC>),
        out_rate: u32,
        note: Note,
    ) -> Self {
        Self {
            sample: sample.1,
            meta_data: sample.0,
            position: (SampleData::PAD_SIZE_EACH, 0.),
            out_rate,
            step_size: Self::compute_step_size(
                sample.0.sample_rate,
                out_rate,
                sample.0.base_note,
                note,
            ),
            note,
        }
    }

    pub fn check_position(&self) -> ControlFlow<()> {
        if self.position.0 > self.sample.deref().len_with_pad() - SampleData::PAD_SIZE_EACH {
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(())
        }
    }

    #[inline]
    fn compute_step_size(
        in_rate: u32,
        out_rate: u32,
        sample_base_note: Note,
        playing_note: Note,
    ) -> f32 {
        // original formula: (outrate / inrate) * (playing_freq / sample_base_freq).
        // Where each freq is computed with MIDI tuning standard formula: 440 * 2^((note - 69)/12)
        // manually reduced formula: 2^((play_note - sample_base_note)/12) * (outrate / inrate)
        // herbie (https://herbie.uwplse.org/demo/index.html) can't optimize further: https://herbie.uwplse.org/demo/e096ef89ee257ad611dd56378bd139a065a6bea0.02e7ec5a3709ad3e06968daa97db50d636f1e44b/graph.html
        (f32::from(i16::from(playing_note.get()) - i16::from(sample_base_note.get())) / 12.).exp2() * (out_rate as f32 / in_rate as f32)
    }

    fn set_step_size(&mut self) {
        self.step_size = Self::compute_step_size(
            self.meta_data.sample_rate,
            self.out_rate,
            self.meta_data.base_note,
            self.note,
        );
    }

    pub fn set_out_samplerate(&mut self, samplerate: u32) {
        self.out_rate = samplerate;
        self.set_step_size();
    }

    fn step(&mut self) {
        self.position.1 += self.step_size;
        let floor = self.position.1.trunc();
        self.position.1 -= floor;
        self.position.0 += floor as usize;
    }

    #[inline]
    pub fn iter<'player, const INTERPOLATION: u8>(
        &'player mut self,
    ) -> SampleIter<'sample, 'player, GC, INTERPOLATION> {
        SampleIter { inner: self }
    }

    #[inline]
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
                let diff = Frame::from(stereo[self.position.0 + 1]) - Frame::from(stereo[self.position.0]);

                (diff * self.position.1) + Frame::from(stereo[self.position.0])
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

pub struct SampleIter<'sample, 'player, const GC: bool, const INTERPOLATION: u8> {
    inner: &'player mut SamplePlayer<'sample, GC>,
}

impl<const GC: bool, const INTERPOLATION: u8> Iterator for SampleIter<'_, '_, GC, INTERPOLATION> {
    type Item = Frame;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next::<INTERPOLATION>()
    }
}
