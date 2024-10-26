#[derive(Debug, Default, Copy, Clone)]
pub enum NoteCommand {
    #[default]
    None, // _, 0
    SetTempo(u8),    // A, 1
    JumpToOrder(u8), // B, 2
    BreakToRow(u8),  // C, 3
    /// Has lot of extra effects depending on value
    VolumeSlideDown(u8), // D, 4
    PitchSlideDown(u8), // E, 5
    PitchSlideUp(u8), // F, 6
    SlideToNote(u8), // G, 7
    Vibrato(u8),     // H, 8
    Tremor(u8),      // I, 9
    Arpeggio(u8),    // J, 10
    VibratoAndVolSlideDown(u8), // K, 11
    SlideToNoteAndVolSlideDown(u8), // L, 12
    SetChannelVol(u8), // M, 13
    /// Some extra effects depending on value
    ChannelVolumeSlideDown(u8), // N, 14
    SetSampleOffset(u8), // O. 15
    /// also can do fine panning
    PanningSlide(u8), // P, 16
    RetriggerNote(u8), // Q, 17
    Tremolo(u8),     // R, 18
    /// Can do a lot of stuff, most of which doesn't have a value
    AlmostEverything(u8), // S, 19
    /// Can also do slides
    TempoChange(u8), // T, 20
    FineVibrato(u8), // U, 21
    SetGlobalVolume(u8), // V, 22
    GlobalVolumeSlide(u8), // W, 23
    SetPanning(u8),  // X, 24
    Panbrello(u8),   // Y, 25
    MIDIMacros(u8),  // Z, 26
                     // Effect byte value reaches until 31, so some missing?
}

pub struct UnknownCommand;

impl TryFrom<(u8, u8)> for NoteCommand {
    type Error = UnknownCommand;

    fn try_from((command_type, command_value): (u8, u8)) -> Result<Self, Self::Error> {
        match command_type {
            0 => Ok(Self::None),
            1 => Ok(Self::SetTempo(command_value)),
            2 => Ok(Self::JumpToOrder(command_value)),
            3 => Ok(Self::BreakToRow(command_value)),
            4 => Ok(Self::VolumeSlideDown(command_value)),
            5 => Ok(Self::PitchSlideDown(command_value)),
            6 => Ok(Self::PitchSlideUp(command_value)),
            7 => Ok(Self::SlideToNote(command_value)),
            8 => Ok(Self::Vibrato(command_value)),
            9 => Ok(Self::Tremor(command_value)),
            10 => Ok(Self::Arpeggio(command_value)),
            11 => Ok(Self::VibratoAndVolSlideDown(command_value)),
            12 => Ok(Self::SlideToNoteAndVolSlideDown(command_value)),
            13 => Ok(Self::SetChannelVol(command_value)),
            14 => Ok(Self::ChannelVolumeSlideDown(command_value)),
            15 => Ok(Self::SetSampleOffset(command_value)),
            16 => Ok(Self::PanningSlide(command_value)),
            17 => Ok(Self::RetriggerNote(command_value)),
            18 => Ok(Self::Tremolo(command_value)),
            19 => Ok(Self::AlmostEverything(command_value)),
            20 => Ok(Self::TempoChange(command_value)),
            21 => Ok(Self::FineVibrato(command_value)),
            22 => Ok(Self::SetGlobalVolume(command_value)),
            23 => Ok(Self::GlobalVolumeSlide(command_value)),
            24 => Ok(Self::SetPanning(command_value)),
            25 => Ok(Self::Panbrello(command_value)),
            26 => Ok(Self::MIDIMacros(command_value)),
            _ => Err(UnknownCommand),
        }
    }
}
