// how to avoid code duplication across Song and GCSong / InternalSong / RealTimeSong
// - trait
//  - no idea how that would look like
//  - would have to be private
//      - are private trait methods available (don't think so)
// - make special samples data type
//  - how???
//  - could be a enum?? with the needed methods for reading and replacing
//      - would need to check size. they would need to be the same
//      - would have to provide easy conversions
//      - bad idea i want to be sure i have a gc version so my clone impl doesn't alloc all of a sudden
//  - some really weird type system struct. (don't think that works at least i can't do it)
// - what code do i even need?? both types are fully transparent anyway. just look inside

use std::array;

use crate::channel::Pan;
use crate::file::impulse_format::header::PatternOrder;
use crate::sample::{Sample, SampleData, SampleMetaData};
use crate::song::pattern::Pattern;
use basedrop::Shared;

use super::note_event::NoteEvent;
use super::pattern::InPatternPosition;

#[derive(Clone)]
pub struct Project<const GC: bool> {
    pub song: Song<GC>,
    pub name: String,
    pub description: String,
}

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
}

impl From<Song<true>> for Song<false> {
    fn from(value: Song<true>) -> Self {
        value.to_owned()
    }
}

impl Song<false> {
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

impl Song<true> {
    pub fn to_owned(self) -> Song<false> {
        Song {
            global_volume: self.global_volume,
            mix_volume: self.mix_volume,
            initial_speed: self.initial_speed,
            initial_tempo: self.initial_tempo,
            pan_separation: self.pan_separation,
            pitch_wheel_depth: self.pitch_wheel_depth,
            patterns: self.patterns.clone(),
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

/// Songoperation only represents valid operations. It needs to be checked for validity when creating.
/// this provides better API as wrong inputs can immediately be returned as errors.
/// it is also more efficient as each operation is applied twice, so checking before is less work than checking when applying
#[derive(Clone)]
pub(crate) enum SongOperation {
    SetVolume(usize, u8),
    SetPan(usize, Pan),
    SetSample(usize, SampleMetaData, Shared<SampleData>),
    SetNoteEvent(usize, InPatternPosition, NoteEvent),
    DeleteNoteEvent(usize, InPatternPosition),
    SetOrder(usize, PatternOrder),
}

impl simple_left_right::Absorb<SongOperation> for Song<true> {
    fn absorb(&mut self, operation: SongOperation) {
        match operation {
            SongOperation::SetVolume(chan, val) => self.volume[chan] = val,
            SongOperation::SetPan(chan, val) => self.pan[chan] = val,
            SongOperation::SetSample(sample, meta, data) => {
                self.samples[sample] = Some((meta, Sample::<true>::new(data)))
            }
            SongOperation::SetNoteEvent(pattern, position, event) => {
                self.patterns[pattern].set_event(position, event)
            }
            SongOperation::DeleteNoteEvent(pattern, position) => {
                self.patterns[pattern].remove_event(position)
            }
            SongOperation::SetOrder(idx, order) => self.pattern_order[idx] = order,
        }
    }
}
