use crate::file::err::LoadDefect;
use crate::file::err;
use crate::song::event_command::NoteCommand;
use crate::song::note_event::{Note, NoteEvent, VolumeEffect};
use crate::song::pattern::{InPatternPosition, Pattern};

/// reader should be buffered in some way and not do a syscall on every read call.
/// 
/// This function does a lot of read calls
pub fn load_pattern<R: std::io::Read + std::io::Seek>(reader: &mut R, defect_handler: &mut dyn FnMut(LoadDefect)) -> Result<Pattern, err::LoadErr> {
    const PATTERN_HEADER_SIZE: usize = 8;

    let read_start = reader.stream_position()?;

    let (length, num_rows) = {
        let mut header = [0; PATTERN_HEADER_SIZE];
        reader.read_exact(&mut header)?;
        (u64::from(u16::from_le_bytes([header[0], header[1]])) + PATTERN_HEADER_SIZE as u64, u16::from_le_bytes([header[2], header[3]]))
    };

    // a guarantee given by the impulse tracker "specs"
    if length >= 64_000 {
        return Err(err::LoadErr::Invalid);
    }

    if !(32..=200).contains(&num_rows) {
        return Err(err::LoadErr::Invalid);
    }

    let mut pattern = Pattern::new(num_rows);

    let mut row_num: u16 = 0;

    let mut last_mask = [0; 64];
    let mut last_event = [NoteEvent::default(); 64];

    let mut scratch = [0; 1];

    while row_num < num_rows && reader.stream_position()? - read_start < length {
        
        let channel_variable = scratch[0];

        if channel_variable == 0 {
            row_num += 1;
            continue;
        }

        let channel = (channel_variable - 1) & 63; // 64 channels, 0 based
        let channel_id = usize::from(channel);

        let maskvar = if (channel_variable & 0b10000000) != 0 {
            reader.read_exact(&mut scratch)?;
            let val = scratch[0];
            last_mask[channel_id] = val;
            val
        } else {
            last_mask[channel_id]
        };

        let mut event = NoteEvent::default();

        // Note
        if (maskvar & 0b00000001) != 0 {
            reader.read_exact(&mut scratch)?;
            let note = match Note::new(scratch[0]) {
                Ok(n) => n,
                Err(_) => {
                    defect_handler(LoadDefect::OutOfBoundsValue);
                    Note::default()
                },
            };

            event.note = note;
            last_event[channel_id].note = note;
        }

        // Instrument / Sample
        if (maskvar & 0b00000010) != 0 {
            reader.read_exact(&mut scratch)?;
            let instrument = scratch[0];

            event.sample_instr = instrument;
            last_event[channel_id].sample_instr = instrument;
        }

        // Volume
        if (maskvar & 0b00000100) != 0 {
            reader.read_exact(&mut scratch)?;
            let vol_pan_raw = scratch[0];
            let vol_pan = match vol_pan_raw.try_into() {
                Ok(v) => v,
                Err(_) => {
                    defect_handler(LoadDefect::OutOfBoundsValue);
                    VolumeEffect::default()
                }
            };

            last_event[channel_id].vol = vol_pan;
            event.vol = vol_pan;
        }

        // Effect
        if (maskvar & 0b00001000) != 0 {
            reader.read_exact(&mut scratch)?;
            let command = scratch[0];
            reader.read_exact(&mut scratch)?;
            let cmd_val = scratch[0];

            let cmd = match NoteCommand::try_from((command, cmd_val)) {
                Ok(cmd) => cmd,
                Err(_) => {
                    defect_handler(LoadDefect::OutOfBoundsValue);
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
        Ok(pattern)
    } else {
        Err(err::LoadErr::BufferTooShort)
    }
}