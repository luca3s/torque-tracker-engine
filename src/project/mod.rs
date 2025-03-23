use song::Song;

pub mod event_command;
pub mod note_event;
pub mod pattern;
pub mod song;

#[derive(Clone)]
pub struct Project<const GC: bool> {
    pub song: Song<GC>,
    pub name: String,
    pub description: String,
}
