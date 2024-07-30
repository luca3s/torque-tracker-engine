use std::ops::Deref;

use crate::{
    audio_processing::{sample::SamplePlayer, Frame},
    file::impulse_format::header::PatternOrder,
    sample::{GetSampleRef, SampleData},
    song::song::Song,
};

pub(crate) struct PlaybackState<S: Deref<Target = SampleData>> {
    // all just the current position
    order: usize,
    pattern: usize,
    row: usize,
    // both of these count down
    tick: u8,
    frame: u32,

    samplerate: u32,

    // i don't like to use a specific type of S here. probabyl needs nightly const_generic_expressions
    voices: Box<[Option<SamplePlayer<S>>; VOICES]>,
}

pub const VOICES: usize = 256;

impl<S> PlaybackState<S>
where
    S: Deref<Target = SampleData>,
{
    pub fn new<GS>(song: &Song<GS>, samplerate: u32) -> Self
    where
        GS: for<'a> GetSampleRef<'a>,
    {
        Self {
            order: todo!("get first order with content"),
            pattern: todo!("get value of that order"),
            row: 0,
            tick: song.initial_speed,
            frame: Self::frames_per_tick(samplerate, song.initial_tempo),
            samplerate,
            voices: Box::new(std::array::from_fn(|_| None)),
        }
    }

    pub fn is_finished<GS>(&self, song: &Song<GS>) -> bool
    where
        GS: for<'a> GetSampleRef<'a>,
    {
        match song.pattern_order.get(self.order) {
            None => true, // this is wrong i believe. needs to check Schism Docs again
            Some(PatternOrder::EndOfSong) => true,
            Some(_) => false,
        }
    }

    pub fn iter<'b, const INTERPOLATION: u8, GS>(
        &'b mut self,
        song: &'b Song<GS>,
    ) -> PlaybackIter<'b, INTERPOLATION, S, GS>
    where
        GS: for<'a> GetSampleRef<'a, SampleRef = S>,
    {
        PlaybackIter { state: self, song }
    }

    fn frames_per_tick(samplerate: u32, tempo: u8) -> u32 {
        u32::from(tempo) / (samplerate * 10)
    }
}

pub(crate) struct PlaybackIter<'b, const INTERPOLATION: u8, S, GS>
where
    S: Deref<Target = SampleData>,
    GS: for<'a> GetSampleRef<'a, SampleRef = S>,
{
    state: &'b mut PlaybackState<S>,
    song: &'b Song<GS>,
}

impl<const INTERPOLATION: u8, S, GS> PlaybackIter<'_, INTERPOLATION, S, GS>
where
    S: Deref<Target = SampleData>,
    GS: for<'a> GetSampleRef<'a, SampleRef = S>,
{
    fn step(&mut self) {
        self.state.frame -= 1;

        if self.state.frame != 0 {
            return;
        }
        self.state.frame =
            PlaybackState::<S>::frames_per_tick(self.state.samplerate, self.song.initial_tempo);
        self.state.tick -= 1;

        if self.state.tick != 0 {
            return;
        }
        self.state.tick = self.song.initial_speed;
        self.state.row += 1;
        // update row: get new note events
        // check for end of pattern
    }

    fn set_instr(&mut self) {
        let (meta, data) = self.song.samples[0].as_ref().unwrap();
        self.state.voices[0] = Some(SamplePlayer::new(
            (*meta, data.get_sample_ref()),
            self.state.samplerate,
            meta.sample_rate,
        ))
    }
}

impl<const INTERPOLATION: u8, S, GS> Iterator for PlaybackIter<'_, INTERPOLATION, S, GS>
where
    S: Deref<Target = SampleData>,
    GS: for<'a> GetSampleRef<'a, SampleRef = S>,
{
    type Item = Frame;

    fn next(&mut self) -> Option<Self::Item> {
        // TODO: make it readable
        let out = self
            .state
            .voices
            .iter_mut()
            .flat_map(|channel| {
                if let Some(voice) = channel {
                    match voice.next::<INTERPOLATION>() {
                        Some(out) => return Some(out),
                        None => *channel = None,
                    }
                }
                None
            })
            .sum();

        self.step();

        Some(out)
    }
}
