use std::ops::ControlFlow;

use crate::{
    audio_processing::{sample::SamplePlayer, Frame},
    file::impulse_format::header::PatternOrder,
    song::{event_command::NoteCommand, song::Song},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlaybackPosition {
    pub order: usize,
    pub pattern: usize,
    pub row: u16,
}

pub struct PlaybackState<'sample, const GC: bool> {
    position: PlaybackPosition,
    // both of these count down
    tick: u8,
    frame: u32,

    current_song_speed: u8,

    samplerate: u32,

    voices: Box<[Option<SamplePlayer<'sample, GC>>; PlaybackState::<true>::VOICES]>,
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

    pub fn get_position(&self) -> PlaybackPosition {
        self.position
    }

    fn process_command(&mut self, command: &NoteCommand) {
        match command {
            NoteCommand::None => (),
            NoteCommand::SetSongSpeed(s) => self.current_song_speed = *s,
        }
    }
}

macro_rules! new {
    ($song:ident, $samplerate:ident) => {{
        let mut order = 0;
        let pattern = $song.next_pattern(&mut order)?;
        let position = PlaybackPosition {
            order,
            pattern: usize::from(pattern),
            row: 0,
        };

        let mut out = Self {
            position,
            tick: $song.initial_speed,
            frame: Self::frames_per_tick($samplerate, $song.initial_tempo),
            current_song_speed: $song.initial_speed,
            $samplerate,
            voices: Box::new(std::array::from_fn(|_| None)),
        };
        out.iter::<0>($song).create_sample_players();
        Some(out)
    }};
}

impl PlaybackState<'static, true> {
    /// None if the Song doesnt have any pattern in its OrderList
    pub fn new(song: &Song<true>, samplerate: u32) -> Option<Self> {
        new!(song, samplerate)
    }
}

impl<'sample> PlaybackState<'sample, false> {
    /// None if the Song doesnt have any pattern in its OrderList
    pub fn new(song: &'sample Song<false>, samplerate: u32) -> Option<Self> {
        new!(song, samplerate)
    }
}

impl<const GC: bool> std::fmt::Debug for PlaybackState<'_, GC> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PlaybackState")
            .field("position", &self.position)
            .field("tick", &self.tick)
            .field("current_song_speed", &self.current_song_speed)
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
    pub fn check_position(&self) -> ControlFlow<()> {
        match self.song.get_order(self.state.position.order) {
            PatternOrder::Number(_) => ControlFlow::Continue(()),
            PatternOrder::EndOfSong => ControlFlow::Break(()),
            PatternOrder::SkipOrder => ControlFlow::Continue(()),
        }
    }

    pub fn frames_per_tick(&self) -> u32 {
        PlaybackState::<GC>::frames_per_tick(self.state.samplerate, self.song.initial_tempo)
    }

    /// do everything needed for stepping except for putting the samples into the voices.
    /// on true that needs to be done otherwise not.
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

        self.state.position.row += 1;
        if self.state.position.row >= self.song.patterns[self.state.position.pattern].row_count() {
            self.state.position.row = 0;
            // next pattern
            if let Some(pattern) = self.song.next_pattern(&mut self.state.position.order) {
                self.state.position.pattern = usize::from(pattern)
            } else {
                return false;
            }
        }

        true
    }
}

/// While the Code is completely identical the types and functions are different.
/// The compiler inferes the right types for each macro call
macro_rules! create_sample_players {
    ($sel:ident) => {
        for (positions, event) in &$sel.song.patterns[$sel.state.position.pattern][$sel.state.position.row] {
            if let Some((meta, sample)) = &$sel.song.samples[usize::from(event.sample_instr)] {
                let player = SamplePlayer::new(
                    (*meta, sample.get_ref()),
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
        if $sel.check_position().is_break() {
            return None;
        }

        let out = $sel
            .state
            .voices
            .iter_mut()
            .flat_map(|channel| {
                if let Some(voice) = channel {
                    match voice.next::<INTERPOLATION>() {
                        Some(frame) => Some(frame),
                        None => {
                            *channel = None;
                            None
                        }
                    }
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

impl<'song, const INTERPOLATION: u8> PlaybackIter<'song, 'song, '_, INTERPOLATION, false> {
    fn step(&mut self) {
        if self.step_generic() {
            self.create_sample_players();
        }
    }
    fn create_sample_players(&mut self) {
        create_sample_players!(self)
    }
}

impl<'song, const INTERPOLATION: u8> Iterator
    for PlaybackIter<'song, 'song, '_, INTERPOLATION, false>
{
    type Item = Frame;

    fn next(&mut self) -> Option<Self::Item> {
        next!(self)
    }
}
