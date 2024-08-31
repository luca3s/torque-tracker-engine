use crate::file::err::LoadDefects;
use crate::file::{err, FileReader};
use crate::song::event_command::NoteCommand;
use crate::song::note_event::{NoteEvent, VolumeEffect};
use crate::song::pattern::{InPatternPosition, Pattern};
use enumflags2::BitFlags;

pub fn load_pattern(buf: &[u8]) -> Result<(Pattern, BitFlags<LoadDefects>), err::LoadErr> {
    const PATTERN_HEADER_SIZE: usize = 8;

    if buf.len() < PATTERN_HEADER_SIZE {
        return Err(err::LoadErr::BufferTooShort);
    }
    let length = usize::from(u16::from_le_bytes([buf[0], buf[1]])) + PATTERN_HEADER_SIZE;
    if buf.len() < length {
        return Err(err::LoadErr::BufferTooShort);
    }
    // a guarantee given by the impulse tracker "specs"
    if length >= 64_000 {
        return Err(err::LoadErr::Invalid);
    }
    let num_rows_header = u16::from_le_bytes([buf[2], buf[3]]);
    if !(32..=200).contains(&num_rows_header) {
        return Err(err::LoadErr::Invalid);
    }

    let mut pattern = Pattern::new(num_rows_header);

    let mut read_pointer: usize = PATTERN_HEADER_SIZE;
    let mut row_num: u16 = 0;
    let mut defects = BitFlags::empty();

    let mut last_mask = [0; 64];
    let mut last_event = [NoteEvent::default(); 64];

    while row_num < num_rows_header && read_pointer < length {
        let channel_variable = *buf.get(read_pointer).ok_or(err::LoadErr::BufferTooShort)?;
        read_pointer += 1;

        if channel_variable == 0 {
            row_num += 1;
            continue;
        }

        let channel = (channel_variable - 1) & 63; // 64 channels, 0 based
        let channel_id = usize::from(channel);

        let maskvar = if (channel_variable & 0b10000000) != 0 {
            let val = *buf.get(read_pointer).ok_or(err::LoadErr::BufferTooShort)?;
            last_mask[channel_id] = val;
            read_pointer += 1;
            val
        } else {
            last_mask[channel_id]
        };

        let mut event = NoteEvent::default();

        // Note
        if (maskvar & 0b00000001) != 0 {
            let note = *buf.get(read_pointer).ok_or(err::LoadErr::BufferTooShort)?;
            read_pointer += 1;

            event.note = note;
            last_event[channel_id].note = note;
        }

        // Instrument / Sample
        if (maskvar & 0b00000010) != 0 {
            let instrument = *buf.get(read_pointer).ok_or(err::LoadErr::BufferTooShort)?;
            read_pointer += 1;

            event.sample_instr = instrument;
            last_event[channel_id].sample_instr = instrument;
        }

        // Volume
        if (maskvar & 0b00000100) != 0 {
            let vol_pan_raw = *buf.get(read_pointer).ok_or(err::LoadErr::BufferTooShort)?;
            read_pointer += 1;
            let vol_pan = match vol_pan_raw.try_into() {
                Ok(v) => v,
                Err(_) => {
                    defects.insert(LoadDefects::OutOfBoundsValue);
                    VolumeEffect::default()
                }
            };

            last_event[channel_id].vol = vol_pan;
            event.vol = vol_pan;
        }

        // Effect
        if (maskvar & 0b00001000) != 0 {
            let command = *buf.get(read_pointer).ok_or(err::LoadErr::BufferTooShort)?;
            read_pointer += 1;
            let cmd_val = *buf.get(read_pointer).ok_or(err::LoadErr::BufferTooShort)?;
            read_pointer += 1;

            let cmd = match NoteCommand::try_from((command, cmd_val)) {
                Ok(cmd) => cmd,
                Err(_) => {
                    defects.insert(LoadDefects::UnknownEffect);
                    NoteCommand::default()
                }
            };
            last_event[channel_id].command = cmd;

            event.command = cmd;
        }

        // Same note
        if (maskvar & 0b00010000) != 0 {
            event.note = last_event[channel_id].note;
        }

        // Same Instr / Sample
        if (maskvar & 0b00100000) != 0 {
            event.sample_instr = last_event[channel_id].sample_instr;
        }

        // Same volume
        if (maskvar & 0b01000000) != 0 {
            event.vol = last_event[channel_id].vol;
        }

        // Same Command
        if (maskvar & 0b10000000) != 0 {
            event.command = last_event[channel_id].command;
        }

        pattern.set_event(
            InPatternPosition {
                row: row_num,
                channel,
            },
            event,
        );
    }

    if pattern.row_count() == row_num {
        Ok((pattern, defects))
    } else {
        Err(err::LoadErr::BufferTooShort)
    }
}

// pub fn load_pattern_new(buf: &[u8]) -> Result<(Pattern, BitFlags<LoadDefects>), err::LoadErr> {
//     const PATTERN_HEADER_SIZE: usize = 8;

//     if buf.len() >= 64_000 {
//         return Err(err::LoadErr::Invalid);
//     }

//     let mut reader = FileReader::new(buf);
//     reader.require_remaining(PATTERN_HEADER_SIZE)?;

//     let lenght = usize::from(reader.get_u16()?) + PATTERN_HEADER_SIZE;
//     reader.require_overall(lenght)?;

//     let num_rows_header = reader.get_u16()?;
//     if !(32..=200).contains(&num_rows_header) {
//         return Err(err::LoadErr::Invalid);
//     }

//     let mut pattern = Pattern::new(num_rows_header);

//     let mut row_num = 0;
//     todo!()
// }
