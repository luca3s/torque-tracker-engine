use crate::song::note_event::NoteEvent;
use crate::song::song::Song;

/// Eq and ord impls only compare the row and channel
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
    use crate::song::pattern::InPatternPosition;
    #[test]
    fn position_ord() {
        let one_zero = InPatternPosition { row: 1, channel: 0 };
        let zero_one = InPatternPosition { row: 0, channel: 1 };
        assert!(one_zero > zero_one);
    }
}

// Events are sorted with the row number as first and channel number as second key.
#[derive(Clone, Debug)]
pub struct Pattern {
    rows: u16,
    data: Vec<(InPatternPosition, NoteEvent)>,
}

impl Default for Pattern {
    fn default() -> Self {
        Self::new(Self::DEFAULT_ROWS)
    }
}

impl Pattern {
    const MAX_LEN: u16 = 200;

    const DEFAULT_ROWS: u16 = 64;

    pub fn new(mut len: u16) -> Self {
        if len > Self::MAX_LEN {
            len = Self::MAX_LEN;
        }
        Self {
            rows: len,
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
                &InPatternPosition {
                    row: new_len,
                    channel: 0,
                },
                |(pos, _)| *pos,
            )
            .unwrap_or_else(|i| i);
        self.data.truncate(idx);
        self.rows = new_len;
    }

    /// overwrites the event if the row already has an event for that channel
    pub fn set_event(&mut self, position: InPatternPosition, event: NoteEvent) {
        match self.data.binary_search_by_key(&position, |(pos, _)| *pos) {
            Ok(idx) => self.data[idx].1 = event,
            Err(idx) => self.data.insert(idx, (position, event)),
        }
    }

    /// if there is no event, does nothing
    pub fn remove_event(&mut self, position: InPatternPosition) {
        if let Ok(index) = self.data.binary_search_by_key(&position, |(pos, _)| *pos) {
            self.data.remove(index);
        }
    }

    pub fn get_row_count(&self) -> u16 {
        self.rows
    }

    // fn sort(&mut self) {
    //     self.data.sort_unstable_by_key(|(pos, _)| *pos);
    // }
}

/// assumes the Operations are correct (not out of bounds, ...)
pub enum PatternOperation {
    Load(Box<[Pattern; Song::MAX_PATTERNS]>),
    SetLength {
        pattern: usize,
        new_len: u16,
    },
    SetEvent {
        pattern: usize,
        row: u16,
        channel: u8,
        event: NoteEvent,
    },
    RemoveEvent {
        pattern: usize,
        row: u16,
        channel: u8,
    },
}

// impl left_right::Absorb<PatternOperation> for [Pattern; Song::MAX_PATTERNS] {
//     fn absorb_first(&mut self, operation: &mut PatternOperation, other: &Self) {
//         // don't need it mutable
//         let operation: &PatternOperation = operation;
//         match operation {
//             PatternOperation::Load(patterns) => *self = *patterns.clone(),
//             PatternOperation::SetLength { pattern, new_len } => self[*pattern].set_length(*new_len),
//             PatternOperation::SetEvent {
//                 pattern,
//                 row,
//                 channel,
//                 event,
//             } => self[*pattern].set_event(
//                 InPatternPosition {
//                     row: *row,
//                     channel: *channel,
//                 },
//                 *event,
//             ),
//             PatternOperation::RemoveEvent {
//                 pattern,
//                 row,
//                 channel,
//             } => self[*pattern].remove_event(InPatternPosition {
//                 row: *row,
//                 channel: *channel,
//             }),
//         }
//     }

//     fn sync_with(&mut self, first: &Self) {
//         self.clone_from(first)
//     }
// }
