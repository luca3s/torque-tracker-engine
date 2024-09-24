#[derive(Debug, Default, Copy, Clone)]
pub enum NoteCommand {
    #[default]
    None,
    SetSongSpeed(u8),
}

impl TryFrom<(u8, u8)> for NoteCommand {
    type Error = (u8, u8);

    fn try_from(value: (u8, u8)) -> Result<Self, Self::Error> {
        let (command_type, command_value) = value;
        match command_type {
            0 => Ok(Self::None),
            _ => Err(value),
        }
    }
}
