use crate::file::impulse_format::sample::VibratoWave;

#[derive(Clone, Debug)]
/// samples need to be padded with PAD_SIZE frames at the start and end
pub enum SampleData {
    Mono(Box<[f32]>),
    Stereo(Box<[[f32; 2]]>),
}

impl SampleData {
    pub const MAX_LENGTH: usize = 16000000;
    pub const MAX_RATE: usize = 192000;
    /// this many frames need to be put on the start and the end
    pub const PAD_SIZE_EACH: usize = 4;

    pub fn len_with_pad(&self) -> usize {
        match self {
            SampleData::Mono(m) => m.len(),
            SampleData::Stereo(s) => s.len(),
        }
    }
}

// mono impl
impl FromIterator<f32> for SampleData {
    fn from_iter<T: IntoIterator<Item = f32>>(iter: T) -> Self {
        let iter = iter.into_iter();
        let size_hint = iter.size_hint();
        let mut data = if let Some(upper_bound) = size_hint.1 {
            Vec::with_capacity(upper_bound + (Self::PAD_SIZE_EACH * 2))
        } else {
            Vec::with_capacity(size_hint.0 + (Self::PAD_SIZE_EACH * 2))
        };

        data.extend_from_slice(&[0.; Self::PAD_SIZE_EACH]);
        data.extend(iter);
        data.extend_from_slice(&[0.; Self::PAD_SIZE_EACH]);

        Self::Mono(data.into_boxed_slice())
    }
}

// stereo impl
impl FromIterator<[f32; 2]> for SampleData {
    fn from_iter<T: IntoIterator<Item = [f32; 2]>>(iter: T) -> Self {
        let iter = iter.into_iter();
        let size_hint = iter.size_hint();
        let mut data = if let Some(upper_bound) = size_hint.1 {
            Vec::with_capacity(upper_bound + (Self::PAD_SIZE_EACH * 2))
        } else {
            Vec::with_capacity(size_hint.0 + (Self::PAD_SIZE_EACH * 2))
        };

        data.extend_from_slice(&[[0.; 2]; Self::PAD_SIZE_EACH]);
        data.extend(iter);
        data.extend_from_slice(&[[0.; 2]; Self::PAD_SIZE_EACH]);

        Self::Stereo(data.into_boxed_slice())
    }
}

#[derive(Clone, Debug)]
pub struct SampleMetaData {
    pub default_volume: u8,
    pub global_volume: u8,
    pub default_pan: Option<u8>,
    pub vibrato_speed: u8,
    pub vibrato_depth: u8,
    pub vibrato_rate: u8,
    pub vibrato_waveform: VibratoWave,
    pub sample_rate: u32,
}

// crazy bad impl. not usable like this
impl Default for SampleMetaData {
    fn default() -> Self {
        Self {
            default_volume: Default::default(),
            global_volume: Default::default(),
            default_pan: Default::default(),
            vibrato_speed: Default::default(),
            vibrato_depth: Default::default(),
            vibrato_rate: Default::default(),
            vibrato_waveform: Default::default(),
            sample_rate: Default::default(),
        }
    }
}
