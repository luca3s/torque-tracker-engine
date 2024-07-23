use crate::{
    audio_processing::sample::SamplePlayer, file::impulse_format::header::PatternOrder,
    song::song::InternalSong,
};

pub(crate) struct PlaybackState {
    order: usize,
    pattern: usize,
    row: usize,
    ticks_to_row: u8,
    samples_to_tick: usize,
    voices: Box<[Option<SamplePlayer>; Self::VOICES]>,
}

impl PlaybackState {
    pub const VOICES: usize = 256;

    pub fn is_finished(&self, song: &InternalSong) -> bool {
        match song.pattern_order.get(self.order) {
            None => true,
            Some(PatternOrder::EndOfSong) => true,
            Some(_) => false,
        }
    }

    pub fn fill_buf(&mut self, buf: &mut [[f32; 2]], song: &InternalSong) {
        buf.fill([0., 0.]);

        let samples_per_tick = song.initial_speed;
    }

    pub fn bounce_song(&mut self, song: &InternalSong) -> Box<[[f32; 2]]> {
        let mut buf = Box::new([[0.; 2]; 1024]);
        let mut out = Vec::new();
        while !self.is_finished(song) {
            self.fill_buf(buf.as_mut(), song);
            out.extend_from_slice(buf.as_ref());
        }
        out.into_boxed_slice()
    }

    pub fn iter<'a>(&'a mut self, song: &'a InternalSong) -> PlaybackIter<'a> {
        PlaybackIter { state: self, song }
    }
}

pub(crate) struct PlaybackIter<'a> {
    state: &'a mut PlaybackState,
    song: &'a InternalSong,
}

impl Iterator for PlaybackIter<'_> {
    type Item = [f32; 2];

    fn next(&mut self) -> Option<Self::Item> {
        todo!()
    }
}
