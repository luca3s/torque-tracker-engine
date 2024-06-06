use crate::playback::event_command::Command;
use std::num::NonZeroU8;

#[derive(Clone, Copy, Debug, Default)]
pub struct Event {
    pub note: u8,
    pub instr: u8,
    pub vol: VolumeEffect,
    pub command: Command,
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
