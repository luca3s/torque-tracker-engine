use enumflags2::{BitFlag, BitFlags};
use crate::file::err::{LoadDefects, LoadErr};

#[derive(Debug, Default)]
pub enum VibratoWave {
    #[default]
    Sine = 0,
    RampDown = 1,
    Square = 2,
    Random = 3,
}

impl TryFrom<u8> for VibratoWave {
    type Error = u8;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Sine),
            1 => Ok(Self::RampDown),
            2 => Ok(Self::Square),
            3 => Ok(Self::Random),
            _ => Err(value),
        }
    }
}

#[derive(Debug)]
pub struct ImpulseSampleHeader {
    dos_filename: Box<[u8]>,
    sample_name: String,
    global_volume: u8,
    flags: u8,
    default_volume: u8,
    convert: u8,
    default_pan: u8,
    length: u32,
    loop_start: u32,
    loop_end: u32,
    c5_speed: u32,
    sustain_start: u32,
    sustain_end: u32,
    data_offset: u32,
    vibrato_speed: u8,
    vibrato_depth: u8,
    vibrato_type: VibratoWave,
    vibrato_rate: u8,
}

impl ImpulseSampleHeader {
    const SIZE: usize = 80;

    pub fn load(buf: &[u8]) -> Result<(Self, BitFlags<LoadDefects>), LoadErr> {
        if buf.len() < Self::SIZE {
            return Err(LoadErr::BufferTooShort);
        }
        if buf[0] != b'I' || buf[1] != b'M' || buf[2] != b'P' || buf[3] != b'S' {
            return Err(LoadErr::Invalid);
        }

        let mut defects = LoadDefects::empty();

        let dos_filename = buf[0x4..=0xF].to_vec().into_boxed_slice();
        if buf[0x10] != 0 {
            return Err(LoadErr::Invalid);
        }

        let global_volume = if buf[0x11] > 64 {
            defects.insert(LoadDefects::OutOfBoundsValue);
            64
        } else {
            buf[0x11]
        };

        let flags = buf[0x12];
        let default_volume = buf[0x13];
        let sample_name = {
            let str = buf[0x14..=0x2D].split(|b| *b == 0).next().unwrap().to_vec();
            let str = String::from_utf8(str);
            if str.is_err() {
                defects.insert(LoadDefects::InvalidText);
            }
            str.unwrap_or_default()
        };

        let convert = buf[0x2E];

        let default_pan = buf[0x2F];

        // in samples, not bytes
        let length = u32::from_le_bytes([buf[0x30], buf[0x31], buf[0x32], buf[0x33]]);
        let loop_start = u32::from_le_bytes([buf[0x34], buf[0x35], buf[0x36], buf[0x37]]);
        let loop_end = u32::from_le_bytes([buf[0x38], buf[0x39], buf[0x3A], buf[0x3B]]);

        // bytes per second at c5
        let c5_speed = {
            let speed = u32::from_le_bytes([buf[0x3C], buf[0x3D], buf[0x3E], buf[0x3F]]);
            if speed > 9999999 {
                defects.insert(LoadDefects::OutOfBoundsValue);
                // no idea what is a good default here
                9999999 / 2
            } else {
                speed
            }
        };

        // in samples, not bytes
        let sustain_start = u32::from_le_bytes([buf[0x40], buf[0x41], buf[0x42], buf[0x43]]);
        let sustain_end = u32::from_le_bytes([buf[0x44], buf[0x45], buf[0x46], buf[0x47]]);
        
        let data_offset = u32::from_le_bytes([buf[0x48], buf[0x49], buf[0x4A], buf[0x4B]]);

        let vibrato_speed = if buf[0x4C] > 64 {
            defects.insert(LoadDefects::OutOfBoundsValue);
            32
        } else {
            buf[0x4C]
        };
        
        let vibrato_depth = if buf[0x4D] > 64 {
            defects.insert(LoadDefects::OutOfBoundsValue);
            32
        } else {
            buf[0x4D]
        };

        let vibrato_rate = if buf[0x4E] > 64 {
            defects.insert(LoadDefects::OutOfBoundsValue);
            32
        } else {
            buf[0x4E]
        };
        
        let vibrato_type = {
            let wave = VibratoWave::try_from(buf[0x4F]);
            if wave.is_err() {
                defects.insert(LoadDefects::OutOfBoundsValue);
            }
            wave.unwrap_or_default()
        };

        Ok((Self {
            dos_filename,
            sample_name,
            global_volume,
            flags,
            default_volume,
            convert,
            default_pan,
            length,
            loop_start,
            loop_end,
            c5_speed,
            sustain_start,
            sustain_end,
            data_offset,
            vibrato_speed,
            vibrato_depth,
            vibrato_type,
            vibrato_rate,
        }, defects))
    }
}
