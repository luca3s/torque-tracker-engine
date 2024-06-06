use std::path::Path;

use enumflags2::BitFlags;

use crate::{
    file::impulse_format::{header::ImpulseHeader, pattern::load_pattern},
    playback::{constants::MAX_PATTERNS, pattern::Pattern},
};

use super::err::{self, LoadDefects};

pub fn load_slice(buf: &[u8]) -> Result<BitFlags<LoadDefects>, err::LoadErr> {
    let (header, mut defects) = ImpulseHeader::load_from_buf(&buf)?;

    let mut patterns: [Pattern; MAX_PATTERNS] = [(); MAX_PATTERNS].map(|_| Pattern::default());
    for (offset, pattern) in header.pattern_offsets.iter().zip(patterns.iter_mut()) {
        if let Some(offset) = offset {
            if buf.len() < usize::try_from(offset.get()).unwrap() {
                return Err(err::LoadErr::BufferTooShort);
            }
            let (new_pattern, pattern_defects) =
                load_pattern(&buf[usize::try_from(offset.get()).unwrap()..])?;
            *pattern = new_pattern;
            defects.insert(pattern_defects);
        }
    }

    Ok(defects)

    // if let Some(samples) = &header.sample_offsets {
    //     println!("sample offsets {samples:?}");

    //     for offset in samples.iter() {
    //         let sample = ImpulseSampleHeader::load(&file_buf[usize::try_from(*offset).unwrap()..]);
    //         println!("sample: {sample:?}");
    //     }
    // }

    // let mut patterns: [Pattern; MAX_PATTERNS] = [(); 240].map(|_| Pattern::default());
    // let mut patterns;
    // if let Some(pattern_offsets) = &header.pattern_offsets {
    //     patterns = Vec::with_capacity(pattern_offsets.len());
    //     for offset in pattern_offsets.iter() {
    //         if *offset != 0 {
    //             patterns.push(load_pattern(&file_buf[usize::try_from(*offset).unwrap()..]));
    //         }
    //     }
    // }
}

pub fn load_file(path: &Path) -> Result<BitFlags<LoadDefects>, err::LoadErr> {
    let file_buf = std::fs::read(path)?;
    load_slice(&file_buf)
}
