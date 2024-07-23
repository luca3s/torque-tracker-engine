use crate::file::err;
use crate::file::err::LoadDefects;
use enumflags2::BitFlags;

#[derive(Debug, Default)]
pub enum NewNoteAction {
    #[default]
    Cut = 0,
    Continue = 1,
    NoteOff = 2,
    NoteFade = 3,
}

impl TryFrom<u8> for NewNoteAction {
    type Error = u8;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Cut),
            1 => Ok(Self::Continue),
            2 => Ok(Self::NoteOff),
            3 => Ok(Self::NoteFade),
            _ => Err(value),
        }
    }
}

#[derive(Debug, Default)]
pub enum DuplicateCheckType {
    #[default]
    Off = 0,
    Note = 1,
    Sample = 2,
    Instrument = 3,
}

impl TryFrom<u8> for DuplicateCheckType {
    type Error = u8;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Off),
            1 => Ok(Self::Note),
            2 => Ok(Self::Sample),
            3 => Ok(Self::Instrument),
            _ => Err(value),
        }
    }
}

#[derive(Debug, Default)]
pub enum DuplicateCheckAction {
    #[default]
    Cut = 0,
    NoteOff = 1,
    NoteFade = 2,
}

impl TryFrom<u8> for DuplicateCheckAction {
    type Error = u8;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Cut),
            1 => Ok(Self::NoteOff),
            2 => Ok(Self::NoteFade),
            _ => Err(value),
        }
    }
}

#[derive(Debug)]
pub struct ImpulseInstrument {
    pub dos_file_name: [u8; 12],
    pub new_note_action: NewNoteAction,
    pub duplicate_check_type: DuplicateCheckType,
    pub duplicate_check_action: DuplicateCheckAction,
    pub fade_out: u16,
    pub pitch_pan_seperation: i8,
    pub pitch_pan_center: u8,
    pub global_volume: u8,
    pub default_pan: Option<u8>,
    pub random_volume: u8,
    pub random_panning: u8,
    pub created_with: u16,
    pub number_of_samples: u8,
    pub name: String,
    pub initial_filter_cutoff: u8,
    pub initial_filter_resonance: u8,
    pub midi_channel: u8,
    pub midi_prgram: u8,
    pub midi_bank: u16,
    pub note_sample_table: [(u8, u8); 120],
    pub envelopes: [ImpulseEnvelope; 3],
}

impl ImpulseInstrument {
    /// size in file is 554 bytes
    pub fn load(buf: &[u8]) -> Result<(Self, BitFlags<err::LoadDefects>), err::LoadErr> {
        if buf.len() < 547 {
            return Err(err::LoadErr::BufferTooShort);
        }
        if buf[0] != b'I' || buf[1] != b'M' || buf[2] != b'P' || buf[3] != b'I' {
            return Err(err::LoadErr::Invalid);
        }

        let mut defects = BitFlags::empty();
        // unwrap is okay as the slice length is const
        let dos_file_name: [u8; 12] = buf[0x04..=0x0F].try_into().unwrap();

        if buf[10] != 0 {
            return Err(err::LoadErr::Invalid);
        }
        let new_note_action = match NewNoteAction::try_from(buf[0x11]) {
            Ok(nna) => nna,
            Err(_) => {
                defects.insert(LoadDefects::OutOfBoundsValue);
                NewNoteAction::default()
            }
        };
        let duplicate_check_type = match DuplicateCheckType::try_from(buf[0x12]) {
            Ok(dct) => dct,
            Err(_) => {
                defects.insert(err::LoadDefects::OutOfBoundsValue);
                DuplicateCheckType::default()
            }
        };
        let duplicate_check_action = match DuplicateCheckAction::try_from(buf[0x13]) {
            Ok(dca) => dca,
            Err(_) => {
                defects.insert(err::LoadDefects::OutOfBoundsValue);
                DuplicateCheckAction::default()
            }
        };
        let fade_out = u16::from_le_bytes([buf[0x14], buf[0x15]]);
        let pitch_pan_seperation = {
            let tmp = i8::from_le_bytes([buf[0x16]]);
            if !(-32..=32).contains(&tmp) {
                defects.insert(err::LoadDefects::OutOfBoundsValue);
                0
            } else {
                tmp
            }
        };
        let pitch_pan_center = if buf[0x17] <= 119 {
            buf[0x17]
        } else {
            defects.insert(err::LoadDefects::OutOfBoundsValue);
            59
        };
        let global_volume = if buf[0x18] <= 128 {
            buf[0x18]
        } else {
            defects.insert(err::LoadDefects::OutOfBoundsValue);
            64
        };

        let default_pan = if buf[0x19] == 128 {
            None
        } else if buf[0x19] > 64 {
            defects.insert(err::LoadDefects::OutOfBoundsValue);
            Some(32)
        } else {
            Some(buf[0x19])
        };

        let random_volume = buf[0x1A];
        assert!(random_volume <= 100);
        let random_pan = buf[0x1B];
        assert!(random_pan <= 100);
        let created_with = u16::from_le_bytes([buf[0x1C], buf[0x1D]]);
        let num_of_samples = buf[0x1E];

        let name = String::from_utf8(
            buf[0x20..=0x39]
                .split(|b| *b == 0)
                .next()
                .unwrap()
                .to_owned(),
        )
        .unwrap();

        let inital_filter_cutoff = buf[0x3A];
        let initial_filter_resonance = buf[0x3B];
        let midi_channel = buf[0x3C];
        let midi_program = buf[0x3D];
        let midi_bank = u16::from_le_bytes([buf[0x3E], buf[0x3F]]);
        let note_sample_table: [(u8, u8); 120] = buf[0x030..0x130]
            .chunks_exact(2)
            .map(|chunk| (chunk[0], chunk[1]))
            .collect::<Vec<(u8, u8)>>()
            .try_into()
            .unwrap();

        let volume_envelope = ImpulseEnvelope::load(&buf[0x130..0x182])?;
        let pan_envelope = ImpulseEnvelope::load(&buf[0x182..0x1D4])?;
        let pitch_envelope = ImpulseEnvelope::load(&buf[0x1D4..])?;

        todo!()
    }
}

/// flags and node values are interpreted differently depending on the type of envelope.
/// doesn't affect loading
#[derive(Debug)]
pub struct ImpulseEnvelope {
    flags: u8,
    num_node_points: u8,
    loop_start: u8,
    loop_end: u8,
    sustain_loop_start: u8,
    sustain_loop_end: u8,
    nodes: [(u8, u16); 25],
}

impl ImpulseEnvelope {
    pub fn load(buf: &[u8]) -> Result<Self, err::LoadErr> {
        if buf.len() < 82 {
            return Err(err::LoadErr::BufferTooShort);
        }

        let flags = buf[0];
        let num_node_points = buf[2];
        let loop_start = buf[3];
        let loop_end = buf[4];
        let sustain_loop_start = buf[5];
        let sustain_loop_end = buf[6];

        // chunks_exact leaves remainder of one in this case but the value of that bit isn't being used
        let nodes: [(u8, u16); 25] = buf[7..]
            .chunks_exact(3)
            .take(25)
            .map(|chunk| (chunk[0], u16::from_le_bytes([chunk[1], chunk[2]])))
            .collect::<Vec<(u8, u16)>>()
            .try_into()
            .unwrap();

        Ok(Self {
            flags,
            num_node_points,
            loop_start,
            loop_end,
            sustain_loop_start,
            sustain_loop_end,
            nodes,
        })
    }
}
