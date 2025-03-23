use err::{LoadDefect, LoadErr};
use impulse_format::{header, pattern};

use crate::project::song::Song;

pub mod err;
pub mod impulse_format;

#[derive(Debug, Clone, Copy)]
pub struct InFilePtr(pub(crate) std::num::NonZeroU32);

impl InFilePtr {
    /// Move the Read Cursor to the value of the file ptr
    pub fn move_to_self<S: std::io::Seek>(self, seeker: &mut S) -> Result<(), std::io::Error> {
        seeker
            .seek(std::io::SeekFrom::Start(self.0.get().into()))
            .map(|_| ())
    }
}

/// Default parsing of a song. Should be fine for most usecases. If you want more customization use the different parsing functions directly.
///
/// R should be buffered in some way and not do a syscall on every read.
/// If you ever find yourself using multiple different reader and/or handlers please open an issue on Github, i will change this to take &dyn.
pub fn parse_song<R: std::io::Read + std::io::Seek>(
    reader: &mut R,
) -> Result<Song<false>, LoadErr> {
    //ignore defects
    let mut defect_handler = |_| ();
    let header = header::ImpulseHeader::parse(reader, &mut defect_handler)?;
    let mut song = Song::default();
    song.copy_values_from_header(&header);

    // parse patterns
    for (idx, ptr) in header
        .pattern_offsets
        .iter()
        .enumerate()
        .flat_map(|(idx, ptr)| ptr.map(|ptr| (idx, ptr)))
    {
        ptr.move_to_self(reader)?;
        let pattern = pattern::parse_pattern(reader, &mut defect_handler)?;
        song.patterns[idx] = pattern;
    }

    Ok(song)
}
