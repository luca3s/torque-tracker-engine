use std::num::NonZeroU32;

use crate::file::{err::{LoadDefect, LoadErr}, InFilePtr};

use super::header;

#[derive(Debug, Default, Clone, Copy)]
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

enum ImpulseSampleBitWidth {
    Seven,
    Eight,
    Sixteen,
    TwentyFour,
    ThirtyTwo,
}

impl From<ImpulseSampleBitWidth> for usize {
    fn from(value: ImpulseSampleBitWidth) -> Self {
        match value {
            ImpulseSampleBitWidth::Seven => 7,
            ImpulseSampleBitWidth::Eight => 8,
            ImpulseSampleBitWidth::Sixteen => 16,
            ImpulseSampleBitWidth::TwentyFour => 24,
            ImpulseSampleBitWidth::ThirtyTwo => 32,
        }
    }
}

enum ImpulseSampleChannels {
    Mono,
    StereoInterleaved,
    StereoSplit,
}

enum ImpulseSampleEndianness {
    Little,
    Big,
}

/// don't know what most of them are. Just took them from include/sndfile.h of schism tracker
enum ImpulseSampleEncoding {
    PCMSigned,
    PCMUnsigned,
    PCMDelta,
    IT2_14Comp,
    IT2_15Comp,
    AMS,
    DMF,
    MDL,
    PTM,
    PCM16,
}

/// don't understand what bit 3 is supposed to do
#[derive(Debug, Copy, Clone)]
struct SampleFormatConvert(u8);
impl SampleFormatConvert {
    pub fn samples_signed(&self) -> bool {
        (self.0 & 0x1) != 0
    }
    /// should only be true for very old files (isn't even supported in C schism)
    pub fn is_big_endian(&self) -> bool {
        (self.0 & 0x2) != 0
    }
    /// alternative is PCM samples
    pub fn delta_samples(&self) -> bool {
        (self.0 & 0x4) != 0
    }
    pub fn tx_wave_12bit(&self) -> bool {
        (self.0 & 0x8) != 0
    }
    /// don't know if this also means that the sample is stereo
    pub fn should_show_stereo_prompt(&self) -> bool {
        (self.0 & 0x10) != 0
    }
}

#[derive(Debug, Copy, Clone)]
pub struct SampleFormatFlags(u8);

impl SampleFormatFlags {
    pub fn has_sample(&self) -> bool {
        (self.0 & 0x01) != 0
    }
    pub fn is_16bit(&self) -> bool {
        (self.0 & 0x02) != 0
    }
    pub fn is_8bit(&self) -> bool {
        !self.is_16bit()
    }
    pub fn is_steroe(&self) -> bool {
        (self.0 & 0x04) != 0
    }
    pub fn is_compressed(&self) -> bool {
        (self.0 & 0x08) != 0
    }
    pub fn uses_loop(&self) -> bool {
        (self.0 & 0x10) != 0
    }
    pub fn uses_sustain_loop(&self) -> bool {
        (self.0 & 0x20) != 0
    }
    pub fn ping_pong_loop(&self) -> bool {
        (self.0 & 0x40) != 0
    }
    pub fn forward_loop(&self) -> bool {
        !self.ping_pong_loop()
    }
    pub fn ping_pong_sustain_loop(&self) -> bool {
        (self.0 & 0x80) != 0
    }
    pub fn forward_sustain_loop(&self) -> bool {
        !self.ping_pong_sustain_loop()
    }
}

#[derive(Debug)]
pub struct ImpulseSampleHeader {
    pub dos_filename: Box<[u8]>,
    pub sample_name: String,
    pub global_volume: u8,
    pub flags: SampleFormatFlags,
    pub default_volume: u8,
    pub convert: u8,
    pub default_pan: u8,
    pub length: u32,
    pub loop_start: u32,
    pub loop_end: u32,
    pub c5_speed: u32,
    pub sustain_start: u32,
    pub sustain_end: u32,
    pub data_ptr: InFilePtr,
    pub vibrato_speed: u8,
    pub vibrato_depth: u8,
    pub vibrato_type: VibratoWave,
    pub vibrato_rate: u8,
}

impl ImpulseSampleHeader {
    const SIZE: usize = 80;

    pub fn parse<H: FnMut(LoadDefect)>(buf: &[u8; Self::SIZE], defect_handler: &mut H) -> Result<Self, LoadErr> {
        if !buf.starts_with(b"IMPS") {
            return Err(LoadErr::Invalid);
        }

        let dos_filename = buf[0x4..=0xF].to_vec().into_boxed_slice();
        if buf[0x10] != 0 {
            return Err(LoadErr::Invalid);
        }

        let global_volume = if buf[0x11] > 64 {
            defect_handler(LoadDefect::OutOfBoundsValue);
            64
        } else {
            buf[0x11]
        };

        let flags = SampleFormatFlags(buf[0x12]);
        let default_volume = buf[0x13];
        let sample_name = {
            let str = buf[0x14..=0x2D].split(|b| *b == 0).next().unwrap().to_vec();
            let str = String::from_utf8(str);
            if str.is_err() {
                defect_handler(LoadDefect::InvalidText);
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
                defect_handler(LoadDefect::OutOfBoundsValue);
                // no idea what is a good default here
                9999999 / 2
            } else {
                speed
            }
        };

        // in samples, not bytes
        let sustain_start = u32::from_le_bytes([buf[0x40], buf[0x41], buf[0x42], buf[0x43]]);
        let sustain_end = u32::from_le_bytes([buf[0x44], buf[0x45], buf[0x46], buf[0x47]]);

        let data_ptr = {
            let value = u32::from_le_bytes([buf[0x48], buf[0x49], buf[0x4A], buf[0x4B]]);
            if value < header::ImpulseHeader::BASE_SIZE as u32 {
                return Err(LoadErr::Invalid);
            }
            InFilePtr(NonZeroU32::new(value).unwrap())
        };

        let vibrato_speed = if buf[0x4C] > 64 {
            defect_handler(LoadDefect::OutOfBoundsValue);
            32
        } else {
            buf[0x4C]
        };

        let vibrato_depth = if buf[0x4D] > 64 {
            defect_handler(LoadDefect::OutOfBoundsValue);
            32
        } else {
            buf[0x4D]
        };

        let vibrato_rate = if buf[0x4E] > 64 {
            defect_handler(LoadDefect::OutOfBoundsValue);
            32
        } else {
            buf[0x4E]
        };

        let vibrato_type = {
            let wave = VibratoWave::try_from(buf[0x4F]);
            if wave.is_err() {
                defect_handler(LoadDefect::OutOfBoundsValue);
            }
            wave.unwrap_or_default()
        };

        Ok(Self {
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
            data_ptr,
            vibrato_speed,
            vibrato_depth,
            vibrato_type,
            vibrato_rate,
        })
    }
}
