use crate::file::err;
use crate::file::err::LoadDefects;
use crate::playback::event::{Event, VolumeEffect};
use crate::playback::event_command;
use crate::playback::pattern::{Pattern, PatternPosition};
use enumflags2::BitFlags;

pub fn load_pattern(buf: &[u8]) -> Result<(Pattern, BitFlags<LoadDefects>), err::LoadErr> {
    const PATTERN_HEADER_SIZE: usize = 8;

    if buf.len() < PATTERN_HEADER_SIZE {
        return Err(err::LoadErr::BufferTooShort);
    }
    // byte length of the pattern in the file
    let length = u16::from_le_bytes([buf[0], buf[1]]);
    if buf.len() != usize::from(length) + PATTERN_HEADER_SIZE {
        return Err(err::LoadErr::BufferTooShort);
    }
    if usize::from(length) + PATTERN_HEADER_SIZE >= 64_000 {
        return Err(err::LoadErr::Invalid);
    }
    let num_rows_header = u16::from_le_bytes([buf[2], buf[3]]);
    if num_rows_header < 32 || num_rows_header < 200 {
        return Err(err::LoadErr::Invalid);
    }

    let mut pattern = Pattern::new(num_rows_header);

    let mut read_pointer = 8;
    let mut row_num: u16 = 0;
    let mut defects = BitFlags::empty();

    let mut last_mask = [0; 64];
    let mut last_event = [Event::default(); 64];

    while row_num <= num_rows_header {
        let channel_variable = *buf.get(read_pointer).ok_or(err::LoadErr::BufferTooShort)?;
        read_pointer += 1;

        if channel_variable == 0 {
            row_num += 1;
            continue;
        }

        let channel = (channel_variable - 1) & 63; // 64 channels, 0 based
        let channel_id = usize::from(channel);

        let maskvariable = if (channel_variable & 0b10000000) != 0 {
            let val = *buf.get(read_pointer).ok_or(err::LoadErr::BufferTooShort)?;
            last_mask[channel_id] = val;
            read_pointer += 1;
            val
        } else {
            last_mask[channel_id]
        };

        let mut event = Event::default();

        // Note
        if (maskvariable & 0b00000001) != 0 {
            let note = *buf.get(read_pointer).ok_or(err::LoadErr::BufferTooShort)?;
            read_pointer += 1;

            event.note = note;
            last_event[channel_id].note = note;
        }

        // Instrument / Sample
        if (maskvariable & 0b00000010) != 0 {
            let instrument = *buf.get(read_pointer).ok_or(err::LoadErr::BufferTooShort)?;
            read_pointer += 1;

            event.instr = instrument;
            last_event[channel_id].instr = instrument;
        }

        // Volume
        if (maskvariable & 0b00000100) != 0 {
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
        if (maskvariable & 0b00001000) != 0 {
            let command = *buf.get(read_pointer).ok_or(err::LoadErr::BufferTooShort)?;
            read_pointer += 1;
            let cmd_val = *buf.get(read_pointer).ok_or(err::LoadErr::BufferTooShort)?;
            read_pointer += 1;

            let cmd = match event_command::Command::try_from((command, cmd_val)) {
                Ok(cmd) => cmd,
                Err(_) => {
                    defects.insert(LoadDefects::UnknownEffect);
                    event_command::Command::default()
                }
            };
            last_event[channel_id].command = cmd;

            event.command = cmd;
        }

        // Same note
        if (maskvariable & 0b00010000) != 0 {
            event.note = last_event[channel_id].note;
        }

        // Same Instr / Sample
        if (maskvariable & 0b00100000) != 0 {
            event.instr = last_event[channel_id].instr;
        }

        // Same volume
        if (maskvariable & 0b01000000) != 0 {
            event.vol = last_event[channel_id].vol;
        }

        // Same Command
        if (maskvariable & 0b10000000) != 0 {
            event.command = last_event[channel_id].command;
        }

        pattern.set_event(
            PatternPosition {
                row: row_num,
                channel,
            },
            event,
        );
    }

    if pattern.get_row_count() == row_num {
        Ok((pattern, defects))
    } else {
        Err(err::LoadErr::BufferTooShort)
    }
}
