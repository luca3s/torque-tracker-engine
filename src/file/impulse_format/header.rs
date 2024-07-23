use crate::file::err::{self, LoadDefects};
use std::num::NonZeroUsize;

use crate::channel::Pan;
use enumflags2::{BitFlag, BitFlags};

#[derive(Debug, Default, Clone, Copy)]
pub enum PatternOrder {
    Number(u8),
    #[default]
    EndOfSong,
    NextOrder,
}

impl TryFrom<u8> for PatternOrder {
    type Error = u8;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            255 => Ok(Self::EndOfSong),
            254 => Ok(Self::NextOrder),
            0..=199 => Ok(Self::Number(value)),
            _ => Err(value),
        }
    }
}

#[derive(Debug)]
pub struct ImpulseHeader {
    pub song_name: String,
    pub philight: u16,

    pub created_with: u16,
    pub compatible_with: u16,
    pub flags: u16,
    pub special: u16,

    pub global_volume: u8,
    pub mix_volume: u8,
    pub initial_speed: u8,
    pub initial_tempo: u8,
    pub pan_separation: u8,
    pub pitch_wheel_depth: u8,
    pub message_length: u16,
    pub message_offset: u32,

    pub channel_pan: [Pan; 64],
    pub channel_volume: [u8; 64],

    pub orders: Box<[PatternOrder]>, // length is oder_num

    /// all Offsets are verified to be point outside the header
    /// Invalid offsets are replaced with None, so patterns or orders don't break, because the indexes change
    pub instr_offsets: Box<[Option<NonZeroUsize>]>,
    pub sample_offsets: Box<[Option<NonZeroUsize>]>,
    /// here None could come from the file, which means an empty pattern
    pub pattern_offsets: Box<[Option<NonZeroUsize>]>,
}

// https://github.com/schismtracker/schismtracker/wiki/ITTECH.TXT
impl ImpulseHeader {
    const BASE_SIZE: usize = 192;

    /// Header is stored at the beginning of the File. length isn't constant, but at least 192 bytes
    /// when unable to load specific parts the function tries its best and communicates the failures in the BitFlags return value.
    /// For some problems it wouldn't make sense to return an incomplete Header as so much would be missing. In those cases an Err is returned
    pub fn load_from_buf(buf: &[u8]) -> Result<(Self, BitFlags<LoadDefects>), err::LoadErr> {
        if buf.len() <= Self::BASE_SIZE {
            return Err(err::LoadErr::BufferTooShort);
        }

        if buf[0] != b'I' || buf[1] != b'M' || buf[2] != b'P' || buf[3] != b'M' {
            return Err(err::LoadErr::Invalid);
        }

        let mut defects = LoadDefects::empty();

        let song_name = {
            let str = buf[0x14..=0x2D].split(|b| *b == 0).next().unwrap().to_vec();
            let str = String::from_utf8(str);
            if str.is_err() {
                defects.insert(LoadDefects::InvalidText);
            }
            str.unwrap_or_default()
        };

        let philight = u16::from_le_bytes([buf[0x1E], buf[0x1F]]);

        let order_num = u16::from_le_bytes([buf[0x20], buf[0x21]]);
        let instr_num = u16::from_le_bytes([buf[0x22], buf[0x23]]);
        let sample_num = u16::from_le_bytes([buf[0x24], buf[0x25]]);
        let pattern_num = u16::from_le_bytes([buf[0x26], buf[0x27]]);
        let created_with = u16::from_le_bytes([buf[0x28], buf[0x29]]);
        let compatible_with = u16::from_le_bytes([buf[0x2A], buf[0x2B]]);
        let flags = u16::from_le_bytes([buf[0x2C], buf[0x2D]]);
        let special = u16::from_le_bytes([buf[0x2E], buf[0x2F]]);

        let order_end = Self::BASE_SIZE + usize::from(order_num);
        let instr_end = order_end + usize::from(instr_num) * 4;
        let sample_end = instr_end + usize::from(sample_num) * 4;
        // pattern end = header end
        let header_end = sample_end + usize::from(pattern_num) * 4;

        // last len check
        if buf.len() <= header_end {
            return Err(err::LoadErr::BufferTooShort);
        }

        let global_volume = if buf[0x30] <= 128 {
            buf[0x30]
        } else {
            defects.insert(LoadDefects::OutOfBoundsValue);
            64
        };

        let mix_volume = if buf[0x31] <= 128 {
            buf[0x31]
        } else {
            defects.insert(LoadDefects::OutOfBoundsValue);
            64
        };

        let initial_speed = buf[0x32];
        let initial_tempo = buf[0x33];
        let pan_separation = buf[0x34];
        let pitch_wheel_depth = buf[0x35];
        let message_length = u16::from_le_bytes([buf[0x36], buf[0x37]]);
        let message_offset = u32::from_le_bytes([buf[0x38], buf[0x39], buf[0x3A], buf[0x3B]]);
        let _reserved = u32::from_le_bytes([buf[0x3C], buf[0x3D], buf[0x3E], buf[0x3F]]);

        // can unwrap here, because the length is already checked at the beginning
        let pan_vals: [u8; 64] = buf[0x40..0x80].try_into().unwrap();
        let channel_pan: [Pan; 64] = pan_vals.map(|pan| match Pan::try_from(pan) {
            Ok(pan) => pan,
            Err(_) => {
                defects.insert(LoadDefects::OutOfBoundsValue);
                Pan::default()
            }
        });

        let channel_volume: [u8; 64] = {
            // can unwrap here, because the length is already checked at the beginning
            let mut vols: [u8; 64] = buf[0x80..0xC0].try_into().unwrap();

            vols.iter_mut().for_each(|vol| {
                if *vol > 64 {
                    defects.insert(LoadDefects::OutOfBoundsValue);
                    *vol = 64
                }
            });
            vols
        };

        let orders: Box<[PatternOrder]> = buf[Self::BASE_SIZE..order_end]
            .iter()
            .map(|order| match PatternOrder::try_from(*order) {
                Ok(pat_order) => pat_order,
                Err(_) => {
                    defects.insert(LoadDefects::OutOfBoundsValue);
                    PatternOrder::NextOrder
                }
            })
            .collect();

        let instr_offsets = get_offset_array(&buf[order_end..instr_end], header_end);

        let sample_offsets = get_offset_array(&buf[instr_end..sample_end], header_end);

        let pattern_offsets = get_offset_array(&buf[sample_end..header_end], header_end);

        Ok((
            Self {
                song_name,
                philight,
                created_with,
                compatible_with,
                flags,
                special,
                global_volume,
                mix_volume,
                initial_speed,
                initial_tempo,
                pan_separation,
                pitch_wheel_depth,
                message_length,
                message_offset,
                channel_pan,
                channel_volume,
                orders,
                instr_offsets,
                sample_offsets,
                pattern_offsets,
            },
            defects,
        ))
    }
}

// used to load Pointers to different pieces of the project from the header.
// the only validation that happens is that pointers have to point outside the header
fn get_offset_array(buf: &[u8], header_end: usize) -> Box<[Option<NonZeroUsize>]> {
    buf.chunks_exact(std::mem::size_of::<u32>())
        .map(|chunk| {
            NonZeroUsize::new(
                // try_from conversion can only fail if usize is smaller than 32 bit
                usize::try_from(u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
                    .unwrap(),
            )
            .filter(|value| value.get() > header_end)
        })
        .collect()
}
