use std::{num::NonZero, ops::ControlFlow};

use crate::{
    audio_processing::{sample::SamplePlayer, Frame},
    manager::PlaybackSettings,
    project::song::Song,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlaybackStatus {
    pub position: PlaybackPosition,
    // which sample is playing,
    // which how far along is each sample
    // which channel is playing
    // ...
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlaybackPosition {
    /// changes behaviour on pattern end and loop behaviour
    pub order: Option<u16>,
    pub pattern: u8,
    pub row: u16,
    /// if order is Some this loops the whole song, otherwise it loops the set pattern
    pub loop_active: bool,
}

impl PlaybackPosition {
    #[inline]
    fn step_row(&mut self, song: &Song) -> ControlFlow<()> {
        self.row += 1;
        if self.row >= song.patterns[usize::from(self.pattern)].row_count() {
            // reset row count
            self.row = 0;
            // compute next pattern
            if let Some(order) = &mut self.order {
                // next pattern according to song orderlist
                if let Some(pattern) = song.next_pattern(order) {
                    // song not finished yet
                    self.pattern = pattern;
                    ControlFlow::Continue(())
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
                        self.pattern = pattern;
                        ControlFlow::Continue(())
                    } else {
                        // the song is empty, so playback is stopped
                        ControlFlow::Break(())
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
    fn new(settings: PlaybackSettings, song: &Song) -> Option<Self> {
        match settings {
            PlaybackSettings::Pattern { idx, should_loop } => {
                // doesn't panic. patterns len is a constant
                if idx < u8::try_from(song.patterns.len()).unwrap() {
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
                    pattern,
                    row: 0,
                    loop_active: should_loop,
                })
            }
        }
    }
}

pub struct PlaybackState {
    position: PlaybackPosition,
    is_done: bool,
    // both of these count down
    tick: u8,
    frame: u32,

    // add current state to support Effects
    samplerate: NonZero<u32>,

    voices: [Option<SamplePlayer>; PlaybackState::VOICES],
}

impl PlaybackState {
    // i don't know yet why those would be different. Splitting them up probably be a bit of work.
    pub const VOICES: usize = Song::MAX_CHANNELS;

    pub fn iter<'playback, 'song, const INTERPOLATION: u8>(
        &'playback mut self,
        song: &'song Song,
    ) -> PlaybackIter<'song, 'playback, INTERPOLATION> {
        PlaybackIter { state: self, song }
    }

    fn frames_per_tick(samplerate: NonZero<u32>, tempo: NonZero<u8>) -> u32 {
        // don't ask me why times 2. it just does the same as schism now
        (samplerate.get() * 2) / u32::from(tempo.get())
    }

    pub fn get_status(&self) -> PlaybackStatus {
        // maybe if it gets more fields compute them while playing back and just copy out here
        PlaybackStatus {
            position: self.position,
        }
    }

    pub fn set_samplerate(&mut self, samplerate: NonZero<u32>) {
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

impl PlaybackState {
    /// None if the settings in the order variant don't have any pattern to play
    pub fn new(song: &Song, samplerate: NonZero<u32>, settings: PlaybackSettings) -> Option<Self> {
        let mut out = Self {
            position: PlaybackPosition::new(settings, song)?,
            is_done: false,
            tick: song.initial_speed.get(),
            frame: Self::frames_per_tick(samplerate, song.initial_tempo),
            samplerate,
            voices: std::array::from_fn(|_| None),
        };
        // Interpolation not important here. no interpolating is done. only sampledata is copied
        out.iter::<0>(song).create_sample_players();
        Some(out)
    }
}

impl std::fmt::Debug for PlaybackState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PlaybackState")
            .field("position", &self.position)
            .field("tick", &self.tick)
            .field("frame", &self.frame)
            .field("samplerate", &self.samplerate)
            .finish_non_exhaustive()?;
        write!(
            f,
            "active channels: {}",
            self.voices.iter().filter(|v| v.is_some()).count()
        )
    }
}

pub struct PlaybackIter<'song, 'playback, const INTERPOLATION: u8> {
    state: &'playback mut PlaybackState,
    song: &'song Song,
}

impl<const INTERPOLATION: u8> PlaybackIter<'_, '_, INTERPOLATION> {
    pub fn frames_per_tick(&self) -> u32 {
        PlaybackState::frames_per_tick(self.state.samplerate, self.song.initial_tempo)
    }
}

impl<const INTERPOLATION: u8> Iterator for PlaybackIter<'_, '_, INTERPOLATION> {
    type Item = Frame;

    fn next(&mut self) -> Option<Self::Item> {
        fn scale_vol(vol: u8) -> f32 {
            (vol as f32) / (u8::MAX as f32)
        }

        if self.state.is_done {
            return None;
        }

        assert!(self.song.volume.len() == self.state.voices.len());

        let out: Frame = self
            .state
            .voices
            .iter_mut()
            .zip(self.song.volume)
            .flat_map(|(channel, vol)| {
                if let Some(voice) = channel {
                    // this logic removes the voices as soon as possible
                    let out = voice.next::<INTERPOLATION>().unwrap();
                    if voice.check_position().is_break() {
                        *channel = None;
                    }
                    let channel_vol = scale_vol(vol);
                    Some(out * channel_vol)
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
        self.step();
        let out_vol = scale_vol(self.song.global_volume);
        Some(out * out_vol)
    }
}

impl<const INTERPOLATION: u8> PlaybackIter<'_, '_, INTERPOLATION> {
    fn step(&mut self) {
        // the current speed is a bit off from schism tracker. i don't know why, how much or in which direction.
        if self.state.frame > 0 {
            self.state.frame -= 1;
            return;
        } else {
            self.state.frame = self.frames_per_tick();
        }

        if self.state.tick > 0 {
            self.state.tick -= 1;
            return;
        } else {
            self.state.tick = self.song.initial_speed.get();
        }

        match self.state.position.step_row(self.song) {
            ControlFlow::Continue(_) => self.create_sample_players(),
            ControlFlow::Break(_) => self.state.is_done = true,
        }
    }
    fn create_sample_players(&mut self) {
        for (position, event) in
            &self.song.patterns[usize::from(self.state.position.pattern)][self.state.position.row]
        {
            if let Some((meta, ref sample)) = self.song.samples[usize::from(event.sample_instr)] {
                let player =
                    SamplePlayer::new(sample.clone(), meta, self.state.samplerate, event.note);
                self.state.voices[usize::from(position.channel)] = Some(player);
            }
        }
    }
}
