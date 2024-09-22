use err::{LoadDefect, LoadErr};
use impulse_format::{header, pattern};

use crate::song::song::Song;

pub mod err;
pub mod impulse_format;

#[derive(Debug, Clone, Copy)]
pub struct InFilePtr(pub(crate) std::num::NonZeroU32);

impl InFilePtr {
    /// Move the Read Cursor to the value of the file ptr
    pub fn seek_to_pos<S: std::io::Seek>(self, seeker: &mut S) -> Result<(), std::io::Error>{
        seeker.seek(std::io::SeekFrom::Start(self.0.get().into())).map(|_| ())
    }
}

/// R should be buffered in some way and not do a syscall on every read.
pub fn load_song<R: std::io::Read + std::io::Seek>(reader: &mut R, defect_handler: &mut dyn FnMut(LoadDefect)) -> Result<Song<false>, LoadErr> {
    let header = header::ImpulseHeader::load(reader, defect_handler)?;
    let mut song = Song::default();
    song.copy_values_from_header(&header);

    // load patterns
    for (idx, ptr) in header.pattern_offsets.iter().enumerate().flat_map(|(idx, ptr)| ptr.map(|ptr| (idx, ptr))) {
        ptr.seek_to_pos(reader)?;
        let pattern = pattern::load_pattern(reader, defect_handler)?;
        song.patterns[idx] = pattern;
    }

    Ok(song)
}
