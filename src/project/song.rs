use std::array;
use std::fmt::{Debug, Formatter};
use std::num::NonZero;

use super::pattern::{Pattern, PatternOperation};
use crate::channel::Pan;
use crate::file::impulse_format;
use crate::file::impulse_format::header::PatternOrder;
use crate::manager::Collector;
use crate::sample::{Sample, SampleMetaData};

#[derive(Clone, Debug)]
pub struct Song {
    pub global_volume: u8,
    pub mix_volume: u8,
    /// Speed specifies how many ticks are in one row. This reduces tempo, but increases resolution of some effects.
    pub initial_speed: NonZero<u8>,
    /// Tempo determines how many ticks are in one second with the following formula: tempo/2 = ticks per second.
    pub initial_tempo: NonZero<u8>,
    pub pan_separation: u8,
    pub pitch_wheel_depth: u8,

    pub patterns: [Pattern; Song::MAX_PATTERNS],
    pub pattern_order: [PatternOrder; Song::MAX_ORDERS],
    pub volume: [u8; Song::MAX_CHANNELS],
    pub pan: [Pan; Song::MAX_CHANNELS],
    pub samples: [Option<(SampleMetaData, Sample)>; Song::MAX_SAMPLES_INSTR],
}

impl Song {
    pub const MAX_ORDERS: usize = 256;
    pub const MAX_PATTERNS: usize = 240;
    pub const MAX_SAMPLES_INSTR: usize = 236;
    pub const MAX_CHANNELS: usize = 64;

    /// order value shouldn't be modified outside of this function.
    /// This moves it forward correctly and returns the pattern to be played
    pub fn next_pattern(&self, order: &mut u16) -> Option<u8> {
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
    pub(crate) fn get_order(&self, order: u16) -> PatternOrder {
        self.pattern_order
            .get(usize::from(order))
            .copied()
            .unwrap_or_default()
    }

    /// takes the values that are included in Song from the header and write them to the song.
    ///
    /// Mostly applies to metadata about the song.
    /// Samples, patterns, instruments are not filled as they are not included in the header
    pub fn copy_values_from_header(&mut self, header: &impulse_format::header::ImpulseHeader) {
        self.global_volume = header.global_volume;
        // TODO: figure out if i want to error here or when parsing the header
        self.initial_speed = NonZero::new(header.initial_speed).unwrap();
        self.initial_tempo = NonZero::new(header.initial_tempo).unwrap();
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
        Ok(())
    }
}

impl Default for Song {
    fn default() -> Self {
        Self {
            global_volume: 128,
            mix_volume: Default::default(),
            initial_speed: NonZero::new(6).unwrap(),
            initial_tempo: NonZero::new(125).unwrap(),
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
    SetVolume(u8, u8),
    SetPan(u8, Pan),
    SetSample(u8, SampleMetaData, Sample),
    RemoveSample(u8),
    PatternOperation(u8, PatternOperation),
    SetOrder(u16, PatternOrder),
    SetInitialSpeed(NonZero<u8>),
    SetInitialTempo(NonZero<u8>),
    SetGlobalVol(u8),
}

/// keep in sync with SongOperation
#[derive(Clone, Debug)]
pub(crate) enum ValidOperation {
    SetVolume(u8, u8),
    SetPan(u8, Pan),
    SetSample(u8, SampleMetaData, Sample),
    RemoveSample(u8),
    PatternOperation(u8, PatternOperation),
    SetOrder(u16, PatternOrder),
    SetInitialSpeed(NonZero<u8>),
    SetInitialTempo(NonZero<u8>),
    SetGlobalVol(u8),
}

impl ValidOperation {
    pub(crate) fn new(
        op: SongOperation,
        handle: &mut Collector,
        song: &Song,
    ) -> Result<ValidOperation, SongOperation> {
        let valid = match op {
            SongOperation::SetVolume(c, _) => usize::from(c) < Song::MAX_CHANNELS,
            SongOperation::SetPan(c, _) => usize::from(c) < Song::MAX_CHANNELS,
            SongOperation::SetSample(idx, _, _) => usize::from(idx) < Song::MAX_SAMPLES_INSTR,
            SongOperation::RemoveSample(idx) => usize::from(idx) < Song::MAX_SAMPLES_INSTR,
            SongOperation::PatternOperation(idx, op) => match song.patterns.get(usize::from(idx)) {
                Some(pattern) => pattern.operation_is_valid(&op),
                None => false,
            },
            SongOperation::SetOrder(idx, _) => usize::from(idx) < Song::MAX_ORDERS,
            SongOperation::SetInitialSpeed(_) => true,
            SongOperation::SetInitialTempo(_) => true,
            SongOperation::SetGlobalVol(_) => true,
        };

        if valid {
            Ok(match op {
                SongOperation::SetVolume(c, v) => Self::SetVolume(c, v),
                SongOperation::SetPan(c, pan) => Self::SetPan(c, pan),
                SongOperation::SetSample(i, meta_data, sample) => {
                    handle.add_sample(sample.clone());
                    Self::SetSample(i, meta_data, sample)
                }
                SongOperation::RemoveSample(i) => Self::RemoveSample(i),
                SongOperation::PatternOperation(i, pattern_operation) => {
                    Self::PatternOperation(i, pattern_operation)
                }
                SongOperation::SetOrder(i, pattern_order) => Self::SetOrder(i, pattern_order),
                SongOperation::SetInitialSpeed(s) => Self::SetInitialSpeed(s),
                SongOperation::SetInitialTempo(t) => Self::SetInitialTempo(t),
                SongOperation::SetGlobalVol(v) => Self::SetGlobalVol(v),
            })
        } else {
            Err(op)
        }
    }
}

impl simple_left_right::Absorb<ValidOperation> for Song {
    fn absorb(&mut self, operation: ValidOperation) {
        match operation {
            ValidOperation::SetVolume(i, val) => self.volume[usize::from(i)] = val,
            ValidOperation::SetPan(i, val) => self.pan[usize::from(i)] = val,
            ValidOperation::SetSample(i, meta, sample) => {
                self.samples[usize::from(i)] = Some((meta, sample))
            }
            ValidOperation::RemoveSample(i) => self.samples[usize::from(i)] = None,
            ValidOperation::PatternOperation(i, op) => {
                self.patterns[usize::from(i)].apply_operation(op)
            }
            ValidOperation::SetOrder(i, order) => self.pattern_order[usize::from(i)] = order,
            ValidOperation::SetInitialSpeed(s) => self.initial_speed = s,
            ValidOperation::SetInitialTempo(t) => self.initial_tempo = t,
            ValidOperation::SetGlobalVol(v) => self.global_volume = v,
        }
    }
}
