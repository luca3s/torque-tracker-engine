use std::ops::{Index, IndexMut};

use crate::project::note_event::NoteEvent;
use crate::project::Song;

/// both row and channel are zero based. If this ever changes a lot of the implementations of
/// Pattern need to be changed, because the searching starts working differently
// don't change the Order of fields, as PartialOrd derive depends on it
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct InPatternPosition {
    pub row: u16,
    pub channel: u8,
}

#[cfg(test)]
mod test {
    use crate::project::pattern::InPatternPosition;
    #[test]
    fn position_ord() {
        let one_zero = InPatternPosition { row: 1, channel: 0 };
        let zero_one = InPatternPosition { row: 0, channel: 1 };
        assert!(one_zero > zero_one);
    }
}

#[derive(Clone, Debug)]
pub struct Pattern {
    rows: u16,
    // Events are sorted with InPatternPosition as the key.
    data: Vec<(InPatternPosition, NoteEvent)>,
}

const fn key(data: &(InPatternPosition, NoteEvent)) -> InPatternPosition {
    data.0
}

impl Default for Pattern {
    fn default() -> Self {
        Self::new(Self::DEFAULT_ROWS)
    }
}

impl Pattern {
    pub const MAX_ROWS: u16 = 200;

    pub const DEFAULT_ROWS: u16 = 64;

    /// panics if len larger than 'Self::MAX_LEN'
    pub const fn new(len: u16) -> Self {
        assert!(len <= Self::MAX_ROWS);
        Self {
            rows: len,
            data: Vec::new(),
        }
    }

    /// panics it the new len is larger than 'Self::MAX_LEN'
    /// deletes the data on higher rows
    pub fn set_length(&mut self, new_len: u16) {
        assert!(new_len <= Self::MAX_ROWS);
        // gets the index of the first element of the first row to be removed
        if new_len < self.rows {
            let idx = self.data.partition_point(|(pos, _)| pos.row < new_len);
            self.data.truncate(idx);
        }
        self.rows = new_len;
    }

    /// overwrites the event if the row already has an event for that channel
    /// panics if the row position is larger than current amount of rows
    pub fn set_event(&mut self, position: InPatternPosition, event: NoteEvent) {
        assert!(position.row < self.rows);
        match self.data.binary_search_by_key(&position, key) {
            Ok(idx) => self.data[idx].1 = event,
            Err(idx) => self.data.insert(idx, (position, event)),
        }
    }

    pub fn get_event(&self, index: InPatternPosition) -> Option<&NoteEvent> {
        self.data
            .binary_search_by_key(&index, key)
            .ok()
            .map(|idx| &self.data[idx].1)
    }

    pub fn get_event_mut(&mut self, index: InPatternPosition) -> Option<&mut NoteEvent> {
        self.data
            .binary_search_by_key(&index, key)
            .ok()
            .map(|idx| &mut self.data[idx].1)
    }

    /// if there is no event, does nothing
    pub fn remove_event(&mut self, position: InPatternPosition) {
        if let Ok(index) = self.data.binary_search_by_key(&position, key) {
            self.data.remove(index);
        }
    }

    pub const fn row_count(&self) -> u16 {
        self.rows
    }

    /// Panics if the Operation is invalid
    pub fn apply_operation(&mut self, op: PatternOperation) {
        match op {
            PatternOperation::SetLength { new_len } => self.set_length(new_len),
            PatternOperation::SetEvent { position, event } => self.set_event(position, event),
            PatternOperation::RemoveEvent { position } => self.remove_event(position),
        }
    }

    pub const fn operation_is_valid(&self, op: &PatternOperation) -> bool {
        match op {
            PatternOperation::SetLength { new_len } => *new_len < Self::MAX_ROWS,
            PatternOperation::SetEvent { position, event: _ } => {
                position.row < self.rows && position.channel as usize <= Song::<false>::MAX_CHANNELS
            }
            PatternOperation::RemoveEvent { position: _ } => true,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }
}

impl Index<u16> for Pattern {
    type Output = [(InPatternPosition, NoteEvent)];

    /// # Out of Bounds
    /// Debug: Panic
    ///
    /// Release: Empty slice
    fn index(&self, index: u16) -> &Self::Output {
        // only a debug assert because if out of bounds the output is simply empty
        debug_assert!(index <= self.rows);
        let start_position = self.data.partition_point(|(pos, _)| {
            *pos < InPatternPosition {
                row: index,
                channel: 0,
            }
        });
        // only search after start_position
        let end_position =
            self.data[start_position..self.data.len()].partition_point(|(pos, _)| {
                *pos < InPatternPosition {
                    row: index + 1,
                    channel: 0,
                }
            }) + start_position;
        &self.data[start_position..end_position]
    }
}

impl Index<InPatternPosition> for Pattern {
    type Output = NoteEvent;

    fn index(&self, index: InPatternPosition) -> &Self::Output {
        self.get_event(index).unwrap()
    }
}

impl IndexMut<InPatternPosition> for Pattern {
    fn index_mut(&mut self, index: InPatternPosition) -> &mut Self::Output {
        self.get_event_mut(index).unwrap()
    }
}

#[derive(Debug, Clone, Copy)]
pub enum PatternOperation {
    SetLength {
        new_len: u16,
    },
    SetEvent {
        position: InPatternPosition,
        event: NoteEvent,
    },
    RemoveEvent {
        position: InPatternPosition,
    },
}
