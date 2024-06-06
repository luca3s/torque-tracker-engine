use std::num::NonZeroU8;

use crate::playback::event::Event;
use left_right::new;

use super::constants::{MAX_PATTERNS, MAX_PATTERN_LEN};

/// Eq and ord impls only compare the row and channel
/// both row and channel are zero based. If this ever changes a lot of the implementations of
/// Patter need to be changed, because the searching starts working differently
#[derive(Clone, Copy, Debug)]
pub struct PatternPosition {
    pub row: u16,
    pub channel: u8,
}

impl PartialEq for PatternPosition {
    fn eq(&self, other: &Self) -> bool {
        self.row == other.row && self.channel == other.channel
    }
}

impl Eq for PatternPosition {}

impl PartialOrd for PatternPosition {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for PatternPosition {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        match self.row.cmp(&other.row) {
            core::cmp::Ordering::Equal => {}
            ord => return ord,
        }
        self.channel.cmp(&other.channel)
    }
}

// Events are sorted with the row number as first and channel number as second key.
#[derive(Clone, Debug)]
pub struct Pattern {
    num_rows: u16,
    data: Vec<(PatternPosition, Event)>,
}

impl Default for Pattern {
    fn default() -> Self {
        Self::new(Self::DEFAULT_ROWS)
    }
}

impl Pattern {
    const MAX_LEN: u16 = MAX_PATTERN_LEN;

    const DEFAULT_ROWS: u16 = 64;

    pub fn new(len: u16) -> Self {
        Self {
            num_rows: len,
            data: Vec::new(),
        }
    }

    /// panics it the new len is larger than 'Self::MAX_LEN'
    /// deletes the data on higher rows
    pub fn set_length(&mut self, new_len: u16) {
        assert!(new_len <= Self::MAX_LEN);
        // gets the index of the first element of the first row to be removed
        let idx = self
            .data
            .binary_search_by_key(
                &PatternPosition {
                    row: new_len,
                    channel: 0,
                },
                |(pos, _)| *pos,
            )
            .unwrap_or_else(|i| i);
        self.data.truncate(idx);
        self.num_rows = new_len;
    }

    /// overwrites the event if the row already has an event for that channel
    pub fn set_event(&mut self, position: PatternPosition, event: Event) {
        match self.data.binary_search_by_key(&position, |(pos, _)| *pos) {
            Ok(idx) => self.data[idx].1 = event,
            Err(idx) => self.data.insert(idx, (position, event)),
        }
    }

    /// if there is no event, does nothing
    pub fn remove_event(&mut self, position: PatternPosition) {
        if let Ok(index) = self.data.binary_search_by_key(&position, |(pos, _)| *pos) {
            self.data.remove(index);
        }
    }

    pub fn get_row_count(&self) -> u16 {
        self.num_rows
    }

    // fn sort(&mut self) {
    //     self.data.sort_unstable_by_key(|(pos, _)| *pos);
    // }
}

/// assumes the Operations are correct (not out of bounds, ...)
pub enum PatternOperation {
    Load(Box<[Pattern; MAX_PATTERNS]>),
    SetLength {
        pattern: usize,
        new_len: u16,
    },
    SetEvent {
        pattern: usize,
        row: u16,
        channel: u8,
        event: Event,
    },
    RemoveEvent {
        pattern: usize,
        row: u16,
        channel: u8,
    },
}

impl left_right::Absorb<PatternOperation> for [Pattern; MAX_PATTERNS] {
    fn absorb_first(&mut self, operation: &mut PatternOperation, other: &Self) {
        // don't need it mutable
        let operation: &PatternOperation = operation;
        match operation {
            PatternOperation::Load(patterns) => *self = *patterns.clone(),
            PatternOperation::SetLength { pattern, new_len } => self[*pattern].set_length(*new_len),
            PatternOperation::SetEvent {
                pattern,
                row,
                channel,
                event,
            } => self[*pattern].set_event(
                PatternPosition {
                    row: *row,
                    channel: *channel,
                },
                *event,
            ),
            PatternOperation::RemoveEvent {
                pattern,
                row,
                channel,
            } => self[*pattern].remove_event(PatternPosition {
                row: *row,
                channel: *channel,
            }),
        }
    }

    fn sync_with(&mut self, first: &Self) {
        self.clone_from(first)
    }
}
