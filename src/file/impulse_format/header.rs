use crate::file::err::{self, LoadDefect};
use std::{io::Read, num::NonZeroU32};

use crate::channel::Pan;

use super::InFilePtr;

/// maybe completely wrong
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum PatternOrder {
    Number(u8),
    #[default]
    EndOfSong,
    SkipOrder,
}

impl TryFrom<u8> for PatternOrder {
    type Error = u8;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            255 => Ok(Self::EndOfSong),
            254 => Ok(Self::SkipOrder),
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

    /// all Offsets are verified to be point outside the header.
    /// 
    /// Invalid offsets are replaced with None, so patterns or orders don't break, because the indexes change
    pub instr_offsets: Box<[Option<InFilePtr>]>,
    pub sample_offsets: Box<[Option<InFilePtr>]>,
    /// here None could come from the file, which means an empty pattern
    pub pattern_offsets: Box<[Option<InFilePtr>]>,
}

// https://github.com/schismtracker/schismtracker/wiki/ITTECH.TXT
impl ImpulseHeader {
    pub(crate) const BASE_SIZE: usize = 0xC0; // = 192

    /// Reader position needs to be at the beginning of the Header.
    /// 
    /// Header is stored at the beginning of the File. length isn't constant, but at least 192 bytes
    /// when unable to load specific parts the function tries its best and communicates the failures in the BitFlags return value.
    /// For some problems it wouldn't make sense to return an incomplete Header as so much would be missing. In those cases an Err is returned
    pub fn load<R: Read>(reader: &mut R, defect_handler: &mut dyn FnMut(LoadDefect)) -> Result<Self, err::LoadErr> {
        let base = {
            let mut base = [0; Self::BASE_SIZE];
            reader.read_exact(&mut base)?;
            base
        };

        // verify that the start matches
        if !base.starts_with(b"IMPM")  {
            return Err(err::LoadErr::Invalid);
        }

        let song_name = {
            let str = base[0x4..=0x1D].split(|b| *b == 0).next().unwrap().to_vec();
            let str = String::from_utf8(str);
            if str.is_err() {
                defect_handler(LoadDefect::InvalidText)
            }
            str.unwrap_or_default()
        };

        let philight = u16::from_le_bytes([base[0x1E], base[0x1F]]);

        let order_num = u16::from_le_bytes([base[0x20], base[0x21]]);
        let instr_num = u16::from_le_bytes([base[0x22], base[0x23]]);
        let sample_num = u16::from_le_bytes([base[0x24], base[0x25]]);
        let pattern_num = u16::from_le_bytes([base[0x26], base[0x27]]);
        let created_with = u16::from_le_bytes([base[0x28], base[0x29]]);
        let compatible_with = u16::from_le_bytes([base[0x2A], base[0x2B]]);
        let flags = u16::from_le_bytes([base[0x2C], base[0x2D]]);
        let special = u16::from_le_bytes([base[0x2E], base[0x2F]]);

        let global_volume = if base[0x30] <= 128 {
            base[0x30]
        } else {
            defect_handler(LoadDefect::OutOfBoundsValue);
            64
        };

        let mix_volume = if base[0x31] <= 128 {
            base[0x31]
        } else {
            defect_handler(LoadDefect::OutOfBoundsValue);
            64
        };

        let initial_speed = base[0x32];
        let initial_tempo = base[0x33];
        let pan_separation = base[0x34];
        let pitch_wheel_depth = base[0x35];
        let message_length = u16::from_le_bytes([base[0x36], base[0x37]]);
        let message_offset = u32::from_le_bytes([base[0x38], base[0x39], base[0x3A], base[0x3B]]);
        let _reserved = u32::from_le_bytes([base[0x3C], base[0x3D], base[0x3E], base[0x3F]]);

        // can unwrap here, because the length is already checked at the beginning
        let pan_vals: [u8; 64] = base[0x40..0x80].try_into().unwrap();
        let channel_pan: [Pan; 64] = pan_vals.map(|pan| match Pan::try_from(pan) {
            Ok(pan) => pan,
            Err(_) => {
                defect_handler(LoadDefect::OutOfBoundsValue);
                Pan::default()
            }
        });

        let channel_volume: [u8; 64] = {
            // can unwrap here, because the length is already checked at the beginning
            let mut vols: [u8; 64] = base[0x80..0xC0].try_into().unwrap();

            vols.iter_mut().for_each(|vol| {
                if *vol > 64 {
                    defect_handler(LoadDefect::OutOfBoundsValue);
                    *vol = 64
                }
            });
            vols
        };

        let orders: Box<[PatternOrder]> = {
            let mut data = vec![0; usize::from(order_num)].into_boxed_slice();
            reader.read_exact(&mut data)?;
            data.iter().map(|order| match PatternOrder::try_from(*order) {
                Ok(pat_order) => pat_order,
                Err(_) => {
                    defect_handler(LoadDefect::OutOfBoundsValue);
                    PatternOrder::SkipOrder
                }
            })
            .collect()
        };

        let instr_offsets = {
            let mut data = vec![0; usize::from(instr_num)].into_boxed_slice();
            reader.read_exact(&mut data)?;
            data.chunks_exact(std::mem::size_of::<u32>()).map(|chunk| {
                let value = u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
                if value <= Self::BASE_SIZE as u32 {
                    defect_handler(LoadDefect::OutOfBoundsPtr);
                    None
                } else {
                    // value is larger than Self::BASE_SIZE, so also larger than 0
                    Some(InFilePtr(NonZeroU32::new(value).unwrap()))
                }
            }).collect()
        };

        let sample_offsets = {
            let mut data = vec![0; usize::from(sample_num)].into_boxed_slice();
            reader.read_exact(&mut data)?;
            data.chunks_exact(std::mem::size_of::<u32>()).map(|chunk| {
                let value = u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
                if value <= Self::BASE_SIZE as u32 {
                    defect_handler(LoadDefect::OutOfBoundsPtr);
                    None
                } else {
                    // value is larger than Self::BASE_SIZE, so also larger than 0
                    Some(InFilePtr(NonZeroU32::new(value).unwrap()))
                }
            }).collect()
        };

        let pattern_offsets = {
            let mut data = vec![0; usize::from(pattern_num)].into_boxed_slice();
            reader.read_exact(&mut data)?;
            data.chunks_exact(std::mem::size_of::<u32>()).map(|chunk| {
                let value = u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
                if value == 0 {
                    // None is a valid value and assumed to be an empty pattern
                    None
                } else if value <= Self::BASE_SIZE as u32 {
                    defect_handler(LoadDefect::OutOfBoundsPtr);
                    None
                } else {
                    // value is larger than Self::BASE_SIZE, so also larger than 0
                    Some(InFilePtr(NonZeroU32::new(value).unwrap()))
                }
            }).collect()
        };

        Ok(Self {
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
        })
    }
}
