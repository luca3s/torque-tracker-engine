#[derive(Debug, Default, Copy, Clone)]
pub enum Command {
    #[default]
    None,
}

impl TryFrom<(u8, u8)> for Command {
    type Error = (u8, u8);

    fn try_from(value: (u8, u8)) -> Result<Self, Self::Error> {
        let (command_type, command_value) = value;
        match command_type {
            0 => Ok(Self::None),
            _ => Err(value),
        }
    }
}
