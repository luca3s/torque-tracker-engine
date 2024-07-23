use enumflags2::BitFlags;

use crate::file::impulse_format::{header::ImpulseHeader, pattern::load_pattern};
use crate::song::pattern::Pattern;
use crate::song::song::{InternalSong, Song};

use super::err::{self, LoadDefects};

// pub fn load_slice(buf: &[u8]) -> Result<(BitFlags<LoadDefects>, InternalSong), err::LoadErr> {
//     let (header, mut defects) = ImpulseHeader::load_from_buf(buf)?;
//     println!("{:?}", header);
//     let patterns = [(); Song::MAX_PATTERNS].map(|_| Pattern::default());
//     for (pattern, offset) in patterns.iter_mut().zip(header.pattern_offsets.iter()) {
//         if let Some(offset) = offset {
//             *pattern = load_pattern(&buf[offset.get()..])?.0;
//         }
//     }

//     Ok(defects)

//     // if let Some(samples) = &header.sample_offsets {
//     //     println!("sample offsets {samples:?}");

//     //     for offset in samples.iter() {
//     //         let sample = ImpulseSampleHeader::load(&file_buf[usize::try_from(*offset).unwrap()..]);
//     //         println!("sample: {sample:?}");
//     //     }
//     // }

//     // let mut patterns: [Pattern; MAX_PATTERNS] = [(); 240].map(|_| Pattern::default());
//     // let mut patterns;
//     // if let Some(pattern_offsets) = &header.pattern_offsets {
//     //     patterns = Vec::with_capacity(pattern_offsets.len());
//     //     for offset in pattern_offsets.iter() {
//     //         if *offset != 0 {
//     //             patterns.push(load_pattern(&file_buf[usize::try_from(*offset).unwrap()..]));
//     //         }
//     //     }
//     // }
// }
