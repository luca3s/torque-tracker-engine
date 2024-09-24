use std::fmt::Debug;
use std::ops::{AddAssign, IndexMut};
use std::sync::mpsc::Receiver;

use crate::audio_processing::sample::Interpolation;
use crate::audio_processing::Frame;
use crate::manager::audio_manager::OutputConfig;
use crate::playback::PlaybackPosition;
use crate::song::note_event::NoteEvent;
use crate::song::song::Song;
use crate::{audio_processing::sample::SamplePlayer, playback::PlaybackState};
use cpal::{Sample, SampleFormat};
use simple_left_right::Reader;

pub(crate) struct LiveAudio {
    song: Reader<Song<true>>,
    playback_state: Option<PlaybackState<'static, true>>,
    live_note: Option<SamplePlayer<'static, true>>,
    manager: Receiver<ToWorkerMsg>,
    audio_msg_config: AudioMsgConfig,
    to_app: futures::channel::mpsc::Sender<FromWorkerMsg>,
    config: OutputConfig,
    buffer: Box<[Frame]>,
}

impl LiveAudio {
    const INTERPOLATION: u8 = Interpolation::Linear as u8;

    pub fn new(
        song: Reader<Song<true>>,
        manager: Receiver<ToWorkerMsg>,
        audio_msg_config: AudioMsgConfig,
        to_app: futures::channel::mpsc::Sender<FromWorkerMsg>,
        config: OutputConfig,
    ) -> Self {
        Self {
            song,
            playback_state: None,
            live_note: None,
            manager,
            audio_msg_config,
            to_app,
            config,
            buffer: vec![Frame::default(); config.buffer_size.try_into().unwrap()].into(),
        }
    }

    #[inline]
    /// returns true if work was done
    fn fill_internal_buffer(&mut self) -> bool {
        let song = self.song.lock();

        for event in self.manager.try_iter() {
            match event {
                ToWorkerMsg::StopPlayback => self.playback_state = None,
                ToWorkerMsg::Playback(_) => {
                    self.playback_state = PlaybackState::<true>::new(&song, self.config.sample_rate);
                }
                ToWorkerMsg::PlayEvent(note) => {
                    if let Some(sample) = &song.samples[usize::from(note.sample_instr)] {
                        let sample_player = SamplePlayer::new(
                            (sample.0, sample.1.get_ref()),
                            self.config.sample_rate / 2,
                            note.note
                        );
                        self.live_note = Some(sample_player);
                    }
                }
            }
        }

        if self.live_note.is_none() && self.playback_state.is_none() {
            // no processing todo
            return false;
        }

        // clear buffer from past run
        self.buffer.fill(Frame::default());

        if let Some(live_note) = &mut self.live_note {
            let note_iter = live_note.iter::<{ Self::INTERPOLATION }>();
            self.buffer
                .iter_mut()
                .zip(note_iter)
                .for_each(|(buf, note)| buf.add_assign(note));

            if live_note.check_position().is_break() {
                self.live_note = None;
            }
        }

        if let Some(playback) = &mut self.playback_state {
            let position = playback.get_position();
            let playback_iter = playback.iter::<{ Self::INTERPOLATION }>(&song);
            self.buffer
                .iter_mut()
                .zip(playback_iter)
                .for_each(|(buf, frame)| buf.add_assign(frame));

            if self.audio_msg_config.playback_position && position != playback.get_position() {
                let _ = self.to_app.try_send(FromWorkerMsg::CurrentPlaybackPosition(playback.get_position()));
            }
        }

        true
    }

    /// converts the internal buffer to any possible output format and channel count
    /// sums stereo to mono and fills channels 3 and up with silence
    #[inline]
    fn fill_from_internal<S: cpal::Sample + cpal::SizedSample + cpal::FromSample<f32>>(
        &mut self,
        data: &mut [S],
    ) {
        // convert the internal buffer and move it to the out_buffer
        if self.config.channel_count.get() == 1 {
            data.iter_mut()
                .zip(self.buffer.iter())
                .for_each(|(out, buf)| *out = buf.to_mono().to_sample());
        } else {
            data.chunks_exact_mut(usize::from(self.config.channel_count.get()))
                .map(|frame| frame.split_first_chunk_mut::<2>().unwrap().0)
                .zip(self.buffer.iter())
                .for_each(|(out, buf)| *out = buf.to_sample());
        }
    }

    pub fn get_generic_callback(
        mut self,
    ) -> impl FnMut(&mut cpal::Data, &cpal::OutputCallbackInfo) {
        move |data, info| {
            assert_eq!(
                data.len(),
                usize::try_from(self.config.buffer_size).unwrap()
                    * usize::from(self.config.channel_count.get())
            );

            // actual audio work, if false no work was done
            if !self.fill_internal_buffer() {
                return;
            }

            // convert to the right output format
            match data.sample_format() {
                SampleFormat::I8 => self.fill_from_internal::<i8>(data.as_slice_mut().unwrap()),
                SampleFormat::I16 => self.fill_from_internal::<i16>(data.as_slice_mut().unwrap()),
                SampleFormat::I32 => self.fill_from_internal::<i32>(data.as_slice_mut().unwrap()),
                SampleFormat::I64 => self.fill_from_internal::<i64>(data.as_slice_mut().unwrap()),
                SampleFormat::U8 => self.fill_from_internal::<u8>(data.as_slice_mut().unwrap()),
                SampleFormat::U16 => self.fill_from_internal::<u16>(data.as_slice_mut().unwrap()),
                SampleFormat::U32 => self.fill_from_internal::<u32>(data.as_slice_mut().unwrap()),
                SampleFormat::U64 => self.fill_from_internal::<u64>(data.as_slice_mut().unwrap()),
                SampleFormat::F32 => self.fill_from_internal::<f32>(data.as_slice_mut().unwrap()),
                SampleFormat::F64 => self.fill_from_internal::<f64>(data.as_slice_mut().unwrap()),
                /*
                I want to support all formats. This panic being triggered means that there is a version
                mismatch between cpal and this library.
                */
                _ => panic!("Sample Format not supported."),
            }

            if self.audio_msg_config.buffer_finished {
                let _ = self
                    .to_app
                    .try_send(FromWorkerMsg::BufferFinished(info.timestamp()));
            }
        }
    }

    pub fn get_typed_callback<S: cpal::SizedSample + cpal::FromSample<f32>>(
        mut self,
    ) -> impl FnMut(&mut [S], &cpal::OutputCallbackInfo) {
        move |data, info| {
            assert_eq!(
                data.len(),
                usize::try_from(self.config.buffer_size).unwrap()
                    * usize::from(self.config.channel_count.get())
            );

            if self.fill_internal_buffer() {
                self.fill_from_internal(data);
            }
        }
    }
}

fn sine(output: &mut [[f32; 2]], sample_rate: f32) {
    let mut sample_clock = 0f32;
    for frame in output {
        sample_clock = (sample_clock + 1.) % sample_rate;
        let value = (sample_clock * 440. * 2. * std::f32::consts::PI / sample_rate).sin();
        *frame.index_mut(0) = value;
        *frame.index_mut(1) = value;
    }
}

#[derive(Default)]
pub struct AudioMsgConfig {
    pub buffer_finished: bool,
    pub playback_position: bool,
}

pub(crate) enum ToWorkerMsg {
    StopPlayback,
    // need some way to encode information about pattern / position
    Playback(PlaybackSettings),
    PlayEvent(NoteEvent),
}

pub struct PlaybackSettings {}

#[derive(Debug)]
pub enum FromWorkerMsg {
    BufferFinished(cpal::OutputStreamTimestamp),
    CurrentPlaybackPosition(PlaybackPosition),
    PlaybackStopped,
}
