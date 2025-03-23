use std::ops::ControlFlow;

use crate::{
    audio_processing::{sample::SamplePlayer, Frame},
    manager::PlaybackSettings,
    project::song::Song,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlaybackStatus {
    position: PlaybackPosition,
    // which sample is playing,
    // which how far along is each sample
    // which channel is playing
    // ...
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlaybackPosition {
    /// changes behaviour on pattern end and loop behaviour
    pub order: Option<usize>,
    pub pattern: usize,
    pub row: u16,
    /// if order is Some this loops the whole song, otherwise it loops the set pattern
    pub loop_active: bool,
}

impl PlaybackPosition {
    #[inline]
    fn step_row<const GC: bool>(&mut self, song: &Song<GC>) -> ControlFlow<()> {
        self.row += 1;
        if self.row >= song.patterns[self.pattern].row_count() {
            // reset row count
            self.row = 0;
            // compute next pattern
            if let Some(order) = &mut self.order {
                // next pattern according to song orderlist
                if let Some(pattern) = song.next_pattern(order) {
                    // song not finished yet
                    self.pattern = pattern.into();
                    return ControlFlow::Continue(());
                } else {
                    // song is finished
                    if !self.loop_active {
                        // not looping, therefore break
                        return ControlFlow::Break(());
                    }
                    // the song should loop
                    // need to check if the song is empty now.
                    *order = 0;
                    if let Some(pattern) = song.next_pattern(order) {
                        self.pattern = pattern.into();
                        return ControlFlow::Continue(());
                    } else {
                        // the song is empty, so playback is stopped
                        return ControlFlow::Break(());
                    }
                }
            } else if self.loop_active {
                // the row count was reset, nothing else to do
                return ControlFlow::Continue(());
            } else {
                // no looping, pattern is done
                return ControlFlow::Break(());
            }
        } else {
            // Pattern not done yet
            ControlFlow::Continue(())
        }
    }

    /// if settings specify a pattern pattern always returns Some
    #[inline]
    fn new<const GC: bool>(settings: PlaybackSettings, song: &Song<GC>) -> Option<Self> {
        match settings {
            PlaybackSettings::Pattern { idx, should_loop } => {
                if idx < song.patterns.len() {
                    Some(Self {
                        order: None,
                        pattern: idx,
                        row: 0,
                        loop_active: should_loop,
                    })
                } else {
                    None
                }
            }
            PlaybackSettings::Order {
                mut idx,
                should_loop,
            } => {
                let pattern = song.next_pattern(&mut idx)?;
                Some(Self {
                    order: Some(idx),
                    pattern: pattern.into(),
                    row: 0,
                    loop_active: should_loop,
                })
            }
        }
    }
}

pub struct PlaybackState<'sample, const GC: bool> {
    position: PlaybackPosition,
    is_done: bool,
    // both of these count down
    tick: u8,
    frame: u32,

    // add current state to support Effects
    samplerate: u32,

    voices: [Option<SamplePlayer<'sample, GC>>; PlaybackState::<true>::VOICES],
}

impl<'sample, const GC: bool> PlaybackState<'sample, GC> {
    pub const VOICES: usize = 256;

    pub fn iter<'playback, 'song, const INTERPOLATION: u8>(
        &'playback mut self,
        song: &'song Song<GC>,
    ) -> PlaybackIter<'sample, 'song, 'playback, INTERPOLATION, GC> {
        PlaybackIter { state: self, song }
    }

    fn frames_per_tick(samplerate: u32, tempo: u8) -> u32 {
        (samplerate * 10) / u32::from(tempo)
    }

    pub fn get_status(&self) -> PlaybackStatus {
        // maybe if it gets more fields compute them while playing back and just copy out here
        PlaybackStatus {
            position: self.position,
        }
    }

    pub fn set_samplerate(&mut self, samplerate: u32) {
        self.samplerate = samplerate;
        self.voices
            .iter_mut()
            .flatten()
            .for_each(|voice| voice.set_out_samplerate(samplerate));
    }

    pub fn is_done(&self) -> bool {
        self.is_done
    }
}

macro_rules! new {
    ($song:ident, $samplerate:ident, $settings:ident) => {{
        let mut out = Self {
            position: PlaybackPosition::new($settings, $song)?,
            is_done: false,
            tick: $song.initial_speed,
            frame: Self::frames_per_tick($samplerate, $song.initial_tempo),
            $samplerate,
            voices: std::array::from_fn(|_| None),
        };
        out.iter::<0>($song).create_sample_players();
        Some(out)
    }};
}

impl PlaybackState<'static, true> {
    /// None if the settings in the order variant don't have any pattern to play
    pub(crate) fn new(
        song: &Song<true>,
        samplerate: u32,
        settings: PlaybackSettings,
    ) -> Option<Self> {
        new!(song, samplerate, settings)
    }
}

impl<'sample> PlaybackState<'sample, false> {
    /// None if the settings in the order variant don't have any pattern to play
    pub fn new(
        song: &'sample Song<false>,
        samplerate: u32,
        settings: PlaybackSettings,
    ) -> Option<Self> {
        new!(song, samplerate, settings)
    }
}

impl<const GC: bool> std::fmt::Debug for PlaybackState<'_, GC> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PlaybackState")
            .field("position", &self.position)
            .field("tick", &self.tick)
            .field("frame", &self.frame)
            .field("samplerate", &self.samplerate)
            .finish()?;
        write!(
            f,
            "active channels: {}",
            self.voices.iter().filter(|v| v.is_some()).count()
        )
    }
}

pub struct PlaybackIter<'sample, 'song, 'playback, const INTERPOLATION: u8, const GC: bool> {
    state: &'playback mut PlaybackState<'sample, GC>,
    song: &'song Song<GC>,
}

impl<const INTERPOLATION: u8, const GC: bool> PlaybackIter<'_, '_, '_, INTERPOLATION, GC> {
    pub fn frames_per_tick(&self) -> u32 {
        PlaybackState::<GC>::frames_per_tick(self.state.samplerate, self.song.initial_tempo)
    }

    /// do everything needed for stepping except for putting the samples into the voices.
    /// on true that needs to be done otherwise not.
    /// true also means that the PlaybackPosition has changed.
    /// Also sets state.is_done if needed
    fn step_generic(&mut self) -> bool {
        if self.state.frame > 0 {
            self.state.frame -= 1;
            return false;
        } else {
            self.state.frame = self.frames_per_tick();
        }

        if self.state.tick > 0 {
            self.state.tick -= 1;
            return false;
        } else {
            self.state.tick = self.song.initial_speed;
        }

        match self.state.position.step_row(self.song) {
            ControlFlow::Continue(_) => true,
            ControlFlow::Break(_) => {
                self.state.is_done = true;
                false
            }
        }
    }
}

/// While the Code is completely identical the types and functions are different.
/// The compiler inferes the right types for each macro call
macro_rules! create_sample_players {
    ($sel:ident) => {
        for (positions, event) in
            &$sel.song.patterns[$sel.state.position.pattern][$sel.state.position.row]
        {
            if let Some((meta, sample)) = &$sel.song.samples[usize::from(event.sample_instr)] {
                let player = SamplePlayer::new(
                    (*meta, sample.get_handle()),
                    $sel.state.samplerate,
                    event.note,
                );
                $sel.state.voices[usize::from(positions.channel)] = Some(player);
            }
        }
    };
}

/// same as above macro
macro_rules! next {
    ($sel: ident) => {{
        if $sel.state.is_done {
            return None;
        }

        let out = $sel
            .state
            .voices
            .iter_mut()
            .flat_map(|channel| {
                if let Some(voice) = channel {
                    // this logic frees the voices as soon as possible
                    let out = voice.next::<INTERPOLATION>().unwrap();
                    if voice.check_position().is_break() {
                        *channel = None;
                    }
                    Some(out)
                    // this logic frees the voices one frame later than possible
                    // match voice.next::<INTERPOLATION>() {
                    //     Some(frame) => Some(frame),
                    //     None => {
                    //         *channel = None;
                    //         None
                    //     }
                    // }
                } else {
                    None
                }
            })
            .sum();
        $sel.step();
        Some(out)
    }};
}

impl<const INTERPOLATION: u8> PlaybackIter<'static, '_, '_, INTERPOLATION, true> {
    fn step(&mut self) {
        if self.step_generic() {
            self.create_sample_players();
        }
    }
    fn create_sample_players(&mut self) {
        create_sample_players!(self)
    }
}

impl<const INTERPOLATION: u8> Iterator for PlaybackIter<'static, '_, '_, INTERPOLATION, true> {
    type Item = Frame;

    fn next(&mut self) -> Option<Self::Item> {
        next!(self)
    }
}

impl<'sample, 'song, const INTERPOLATION: u8> PlaybackIter<'sample, 'song, '_, INTERPOLATION, false>
where
    'song: 'sample,
{
    fn step(&mut self) {
        if self.step_generic() {
            self.create_sample_players();
        }
    }
    fn create_sample_players(&mut self) {
        create_sample_players!(self)
    }
}

impl<'sample, 'song, const INTERPOLATION: u8> Iterator
    for PlaybackIter<'sample, 'song, '_, INTERPOLATION, false>
where
    'song: 'sample,
{
    type Item = Frame;

    fn next(&mut self) -> Option<Self::Item> {
        next!(self)
    }
}
