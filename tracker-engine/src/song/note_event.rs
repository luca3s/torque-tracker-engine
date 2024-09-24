use std::fmt::{Display, Write};

use crate::song::event_command::NoteCommand;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Note(u8);

impl Note {
    pub fn new(value: u8) -> Result<Note, u8> {
        if value > 199 {
            Err(value)
        } else {
            Ok(Note(value))
        }
    }

    pub const fn get_octave(self) -> u8 {
        self.0 / 12
    }

    pub const fn get_note_name(self) -> &'static str {
        match self.0 % 12 {
            0 => "C",
            1 => "C#",
            2 => "D",
            3 => "D#",
            4 => "E",
            5 => "F",
            6 => "F#",
            7 => "G",
            8 => "G#",
            9 => "A",
            10 => "A#",
            11 => "B",
            _ => panic!()
        }
    }

    pub fn get_frequency(self) -> f32 {
        // taken from https://en.wikipedia.org/wiki/MIDI_tuning_standard
        440. * ((f32::from(self.0) - 69.) / 12.).exp2()
    }

    pub const fn get(self) -> u8 {
        self.0
    }
}

impl Display for Note {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.get_note_name())?;
        f.write_char('-')?;
        self.get_octave().fmt(f)?;
        Ok(())
    }
}

impl Default for Note {
    fn default() -> Self {
        Self(60) // C-5
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct NoteEvent {
    pub note: Note,
    pub sample_instr: u8,
    pub vol: VolumeEffect,
    pub command: NoteCommand,
}

#[derive(Debug, Clone, Copy, Default)]
pub enum VolumeEffect {
    FineVolSlideUp(u8),
    FineVolSlideDown(u8),
    VolSlideUp(u8),
    VolSlideDown(u8),
    PitchSlideUp(u8),
    PitchSlideDown(u8),
    SlideToNoteWithSpeed(u8),
    VibratoWithSpeed(u8),
    Volume(u8),
    Panning(u8),
    /// Uses Instr / Sample Default Volume
    #[default]
    None,
}

impl TryFrom<u8> for VolumeEffect {
    type Error = u8;

    /// IT Tracker Format Conversion
    /// no way to get None, as then it just doesn't get set
    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0..=64 => Ok(Self::Volume(value)),
            65..=74 => Ok(Self::FineVolSlideUp(value - 65)),
            75..=84 => Ok(Self::FineVolSlideDown(value - 75)),
            85..=94 => Ok(Self::VolSlideUp(value - 85)),
            95..=104 => Ok(Self::VolSlideDown(value - 95)),
            105..=114 => Ok(Self::PitchSlideDown(value - 105)),
            115..=124 => Ok(Self::PitchSlideUp(value - 115)),
            128..=192 => Ok(Self::Panning(value - 128)),
            193..=202 => Ok(Self::SlideToNoteWithSpeed(value - 193)),
            203..=212 => Ok(Self::VibratoWithSpeed(value - 203)),
            _ => Err(value),
        }
    }
}
