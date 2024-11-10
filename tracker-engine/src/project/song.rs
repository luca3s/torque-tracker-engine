use std::array;
use std::fmt::{Debug, Formatter};
use std::ops::Deref;

use crate::channel::Pan;
use crate::file::impulse_format;
use crate::file::impulse_format::header::PatternOrder;
use crate::sample::{Sample, SampleData, SampleMetaData};
use basedrop::Shared;

use super::pattern::{Pattern, PatternOperation};

/// Playback Speed in Schism is determined by two values: Tempo and Speed.
/// Speed specifies how many ticks are in one row. This reduces tempo, but increases resolution of some effects.
/// Tempo determines how many ticks are in one second with the following formula: tempo/10 = ticks per second.
#[derive(Clone, Debug)]
pub struct Song<const GC: bool> {
    pub global_volume: u8,
    pub mix_volume: u8,
    pub initial_speed: u8,
    pub initial_tempo: u8,
    pub pan_separation: u8,
    pub pitch_wheel_depth: u8,

    pub patterns: [Pattern; Song::<true>::MAX_PATTERNS],
    pub pattern_order: [PatternOrder; Song::<true>::MAX_ORDERS],
    pub volume: [u8; Song::<true>::MAX_CHANNELS],
    pub pan: [Pan; Song::<true>::MAX_CHANNELS],
    pub samples: [Option<(SampleMetaData, Sample<GC>)>; Song::<true>::MAX_SAMPLES],
}

impl<const GC: bool> Song<GC> {
    pub const MAX_ORDERS: usize = 256;
    pub const MAX_PATTERNS: usize = 240;
    pub const MAX_SAMPLES: usize = 236;
    pub const MAX_INSTR: usize = Self::MAX_SAMPLES;
    pub const MAX_CHANNELS: usize = 64;

    /// order value shouldn't be modified outside of this function.
    /// This moves it forward correctly and returns the pattern to be played
    pub fn next_pattern(&self, order: &mut usize) -> Option<u8> {
        loop {
            match self.get_order(*order) {
                PatternOrder::Number(pattern) => break Some(pattern),
                PatternOrder::EndOfSong => break None,
                PatternOrder::SkipOrder => (),
            }
            *order += 1;
        }
    }

    /// out of bounds is EndOfSong
    pub(crate) fn get_order(&self, order: usize) -> PatternOrder {
        self.pattern_order.get(order).copied().unwrap_or_default()
    }

    /// takes the values that are included in Song from the header and write them to the song.
    ///
    /// Mostly applies to metadata about the song.
    /// Samples, patterns, instruments are not filled as they are not included in the header
    pub fn copy_values_from_header(&mut self, header: &impulse_format::header::ImpulseHeader) {
        self.global_volume = header.global_volume;
        self.initial_speed = header.initial_speed;
        self.initial_tempo = header.initial_tempo;
        self.mix_volume = header.mix_volume;
        self.pan_separation = header.pan_separation;
        self.pitch_wheel_depth = header.pitch_wheel_depth;

        self.pan = header.channel_pan;
        self.volume = header.channel_volume;

        for (idx, order) in header.orders.iter().enumerate() {
            self.pattern_order[idx] = *order;
        }
    }

    /// debug like impl which isn't as long by cutting down a lot of information
    pub fn dbg_relevant(&self, f: &mut Formatter<'_>) -> Result<(), std::fmt::Error> {
        write!(f, "global_volume: {}, ", self.global_volume)?;
        write!(f, "mix_volume: {}, ", self.mix_volume)?;
        write!(f, "initial_speed: {}, ", self.initial_speed)?;
        write!(f, "initial_tempo: {}, ", self.initial_tempo)?;
        write!(f, "pan_seperation: {}, ", self.pan_separation)?;
        write!(f, "pitch_wheel_depth: {}, ", self.pitch_wheel_depth)?;
        write!(
            f,
            "{} not empty patterns, ",
            self.patterns.iter().filter(|p| !p.is_empty()).count()
        )?;
        write!(
            f,
            "{} orders, ",
            self.pattern_order
                .iter()
                .filter(|o| **o != PatternOrder::EndOfSong)
                .count()
        )?;
        write!(f, "{} samples", self.samples.iter().flatten().count())?;
        Ok(())
    }
}

impl Song<false> {
    // to avoid cloning patterns
    #[expect(clippy::wrong_self_convention)]
    pub(crate) fn to_gc(self, handle: &basedrop::Handle) -> Song<true> {
        Song {
            global_volume: self.global_volume,
            mix_volume: self.mix_volume,
            initial_speed: self.initial_speed,
            initial_tempo: self.initial_tempo,
            pan_separation: self.pan_separation,
            pitch_wheel_depth: self.pitch_wheel_depth,
            patterns: self.patterns,
            pattern_order: self.pattern_order,
            volume: self.volume,
            pan: self.pan,
            samples: self
                .samples
                .map(|s| s.map(|(meta, data)| (meta, data.to_gc(handle)))),
        }
    }
}

impl From<Song<true>> for Song<false> {
    fn from(value: Song<true>) -> Self {
        value.to_owned()
    }
}

impl Song<true> {
    pub fn to_owned(self) -> Song<false> {
        Song {
            global_volume: self.global_volume,
            mix_volume: self.mix_volume,
            initial_speed: self.initial_speed,
            initial_tempo: self.initial_tempo,
            pan_separation: self.pan_separation,
            pitch_wheel_depth: self.pitch_wheel_depth,
            patterns: self.patterns,
            pattern_order: self.pattern_order,
            volume: self.volume,
            pan: self.pan,
            samples: self
                .samples
                .map(|option| option.map(|(meta, data)| (meta, data.to_owned()))),
        }
    }
}

impl<const GC: bool> Default for Song<GC> {
    fn default() -> Self {
        Self {
            global_volume: 128,
            mix_volume: Default::default(),
            initial_speed: 6,
            initial_tempo: 125,
            pan_separation: 128,
            pitch_wheel_depth: Default::default(),
            patterns: array::from_fn(|_| Pattern::default()),
            pattern_order: array::from_fn(|_| PatternOrder::default()),
            volume: array::from_fn(|_| 64),
            pan: array::from_fn(|_| Pan::default()),
            samples: array::from_fn(|_| None),
        }
    }
}

// On change: also change ValidOperation
#[derive(Clone, Debug)]
pub enum SongOperation {
    SetVolume(usize, u8),
    SetPan(usize, Pan),
    SetSample(usize, SampleMetaData, SampleData),
    PatternOperation(usize, PatternOperation),
    SetOrder(usize, PatternOrder),
}

/// keep in sync with SongOperation
#[derive(Clone)]
pub(crate) enum ValidOperation {
    SetVolume(usize, u8),
    SetPan(usize, Pan),
    SetSample(usize, SampleMetaData, Shared<SampleData>),
    PatternOperation(usize, PatternOperation),
    SetOrder(usize, PatternOrder),
}

impl ValidOperation {
    pub(crate) fn new(
        op: SongOperation,
        handle: &basedrop::Handle,
        song: &Song<true>,
    ) -> Result<ValidOperation, SongOperation> {
        let valid = match op {
            SongOperation::SetVolume(c, _) => c < Song::<true>::MAX_CHANNELS,
            SongOperation::SetPan(c, _) => c < Song::<true>::MAX_CHANNELS,
            SongOperation::SetSample(idx, _, _) => idx < Song::<true>::MAX_SAMPLES,
            SongOperation::PatternOperation(idx, op) => match song.patterns.get(idx) {
                Some(pattern) => pattern.operation_is_valid(&op),
                None => false,
            },
            SongOperation::SetOrder(idx, _) => idx < Song::<true>::MAX_ORDERS,
        };

        if valid {
            Ok(match op {
                SongOperation::SetVolume(c, v) => Self::SetVolume(c, v),
                SongOperation::SetPan(c, pan) => Self::SetPan(c, pan),
                SongOperation::SetSample(i, sample_meta_data, sample_data) => {
                    Self::SetSample(i, sample_meta_data, Shared::new(handle, sample_data))
                }
                SongOperation::PatternOperation(i, pattern_operation) => {
                    Self::PatternOperation(i, pattern_operation)
                }
                SongOperation::SetOrder(i, pattern_order) => Self::SetOrder(i, pattern_order),
            })
        } else {
            Err(op)
        }
    }

    pub(crate) fn drops_sample(&self, song: &Song<true>) -> bool {
        if let Self::SetSample(idx, _, _) = self {
            if song.samples[*idx].is_some() {
                // a sample is replaced
                true
            } else {
                // there is no sample at the place of the new sample, so no sample gets dropped
                false
            }
        } else {
            // the operation doesn't set a sample
            false
        }
    }
}

impl Debug for ValidOperation {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SetVolume(arg0, arg1) => {
                f.debug_tuple("SetVolume").field(arg0).field(arg1).finish()
            }
            Self::SetPan(arg0, arg1) => f.debug_tuple("SetPan").field(arg0).field(arg1).finish(),
            Self::SetSample(arg0, arg1, arg2) => f
                .debug_tuple("SetSample")
                .field(arg0)
                .field(arg1)
                .field(arg2.deref())
                .finish(),
            Self::PatternOperation(arg0, arg1) => f
                .debug_tuple("PatternOperation")
                .field(arg0)
                .field(arg1)
                .finish(),
            Self::SetOrder(arg0, arg1) => {
                f.debug_tuple("SetOrder").field(arg0).field(arg1).finish()
            }
        }
    }
}

impl simple_left_right::Absorb<ValidOperation> for Song<true> {
    fn absorb(&mut self, operation: ValidOperation) {
        match operation {
            ValidOperation::SetVolume(chan, val) => self.volume[chan] = val,
            ValidOperation::SetPan(chan, val) => self.pan[chan] = val,
            ValidOperation::SetSample(sample, meta, data) => {
                self.samples[sample] = Some((meta, Sample::<true>::new(data)))
            }
            ValidOperation::PatternOperation(pattern, op) => {
                self.patterns[pattern].apply_operation(op)
            }
            ValidOperation::SetOrder(idx, order) => self.pattern_order[idx] = order,
        }
    }
}
