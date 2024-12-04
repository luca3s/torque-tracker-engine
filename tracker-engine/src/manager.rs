use std::{fmt::Debug, mem::ManuallyDrop, num::NonZeroU16, time::Duration};

use basedrop::Collector;
use cpal::traits::{DeviceTrait, StreamTrait};
use simple_left_right::{WriteGuard, Writer};

use crate::{
    audio_processing::playback::PlaybackPosition,
    live_audio::{LiveAudio, LiveAudioStatus},
    project::{
        note_event::NoteEvent,
        song::{Song, SongOperation, ValidOperation},
    },
};

#[derive(Debug, Clone, Copy)]
pub enum ToWorkerMsg {
    Playback(PlaybackSettings),
    StopPlayback,
    PlayEvent(NoteEvent),
    StopLiveNote,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub enum SendResult {
    Success,
    BufferFull,
    AudioInactive
}

impl SendResult {
    #[track_caller]
    pub fn unwrap(self) {
        match self {
            SendResult::Success => (),
            SendResult::BufferFull => panic!("Buffer full"),
            SendResult::AudioInactive => panic!("Audio inactive"),
        }
    }

    pub fn is_success(self) -> bool {
        self == Self::Success
    }
}

struct ActiveStream {
    stream: cpal::Stream,
    buffer_time: Duration,
    send: rtrb::Producer<ToWorkerMsg>,
    status: triple_buffer::Output<Option<LiveAudioStatus>>,
}

impl Debug for ActiveStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ActiveStream").field("buffer_time", &self.buffer_time).field("send", &self.send).field("status", &self.status).finish()
    }
}

/// You will need to write your own spin loops.
/// For that you can and maybe should use AudioManager::buffer_time.
pub struct AudioManager {
    song: Writer<Song<true>, ValidOperation>,
    gc: std::mem::ManuallyDrop<Collector>,
    stream: Option<ActiveStream>,
}

impl AudioManager {
    pub fn new(song: Song<false>) -> Self {
        let gc = std::mem::ManuallyDrop::new(basedrop::Collector::new());
        let left_right = simple_left_right::Writer::new(song.to_gc(&gc.handle()));

        Self {
            song: left_right,
            gc,
            stream: None,
        }
    }

    pub fn try_edit_song(&mut self) -> Option<SongEdit<'_>> {
        self.song.try_lock().map(|song| SongEdit { song, gc: self.gc.handle() })
    }

    pub fn get_song(&self) -> &Song<true> {
        self.song.read()
    }

    pub fn collect_garbage(&mut self) {
        self.gc.collect();
    }

    /// If the config specifies more than two channels only the first two will be filled with audio.
    /// The rest gets silence.
    pub fn init_audio(
        &mut self,
        device: cpal::Device,
        config: OutputConfig,
    ) -> Result<(), cpal::BuildStreamError> {
        const TO_WORKER_CAPACITY: usize = 5;

        let from_worker = triple_buffer::triple_buffer(&None);
        let to_worker = rtrb::RingBuffer::new(TO_WORKER_CAPACITY);
        let reader = self.song.build_reader().unwrap();

        let audio_worker = LiveAudio::new(reader, to_worker.1, from_worker.0, config);

        let stream = device.build_output_stream_raw(
            &config.into(),
            cpal::SampleFormat::F32,
            audio_worker.get_generic_callback(),
            |err| println!("{err}"),
            None,
        )?;
        let buffer_time =
            Duration::from_millis((config.buffer_size * 1000 / config.buffer_size).into());

        stream.play().unwrap();

        self.stream = Some(ActiveStream { stream, buffer_time, send: to_worker.0, status: from_worker.1 });

        Ok(())
    }

    /// pauses the audio. only works on some platforms (look at cpal docs)
    pub fn pause_audio(&mut self) {
        if let Some(stream) = &mut self.stream {
            stream.stream.pause().unwrap();
        }
    }

    /// resume the audio. playback is in the same state as before the pause. (only available on some platforms, see cpal docs for stream.pause())
    pub fn resume_audio(&self) {
        if let Some(stream) = &self.stream {
            stream.stream.play().unwrap();
        }
    }

    /// None if there is no active stream.
    /// Some(Err) if the buffer is full.
    pub fn try_msg_worker(&mut self, msg: ToWorkerMsg) -> SendResult {
        if let Some(stream) = &mut self.stream {
            match stream.send.push(msg) {
                Ok(_) => SendResult::Success,
                Err(_) => SendResult::BufferFull,
            }
        } else {
            SendResult::AudioInactive
        }
    }

    /// closes the audio backend.
    pub fn deinit_audio(&mut self) {
        self.stream = None;
        self.gc.collect();
    }

    /// last playback status sent by the audio worker
    pub fn playback_status(&mut self) -> Option<&LiveAudioStatus> {
        self.stream
            .as_mut()
            .and_then(|s| s.status.read().as_ref())
    }

    /// Some if a stream is active.
    /// Returns the approximate time it takes to process an audio buffer based on the used settings.
    /// 
    /// Useful for implementing spin_loops on collect_garbage or for locking a SongEdit as every time a buffer is finished
    /// garbage could be releases and a lock could be made available
    pub fn buffer_time(&self) -> Option<Duration> {
        self.stream.as_ref().map(|s| s.buffer_time)
    }
}

impl Debug for AudioManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AudioManager")
            .field("song", &self.song)
            .field("stream", &self.stream)
            .finish()
    }
}

impl Drop for AudioManager {
    /// if this panics the drop implementation isn't right and the Audio Callback isn't cleaned up properly
    fn drop(&mut self) {
        self.deinit_audio();
        let mut song = self.try_edit_song().unwrap();
        for i in 0..Song::<true>::MAX_SAMPLES {
            song.apply_operation(SongOperation::RemoveSample(i))
                .unwrap();
        }
        song.finish();
        // lock it once more to ensure that the changes were propagated
        self.try_edit_song().unwrap();
        let mut gc = unsafe { ManuallyDrop::take(&mut self.gc) };
        gc.collect();
        if let Err(gc) = gc.try_cleanup() {
            assert_eq!(gc.handle_count(), 0, "Bug, as the only time i have a handle is inside a SongEdit, which borrows Manager anyways");
            // to avoid this from happening close the audio before dropping the audio manager
            eprintln!("Audio thread cleanup didn't run before dropping the AudioManager. {} samples were leaked", gc.alloc_count())
        }
    }
}

/// the changes made to the song will be made available to the playing live audio as soon as
/// this struct is dropped.
///
/// With this you can load the full song without ever playing a half initialised state
/// when doing mulitple operations this object should be kept as it is
// should do all the verfication of
// need manuallyDrop because i need consume on drop behaviour
pub struct SongEdit<'a> {
    song: WriteGuard<'a, Song<true>, ValidOperation>,
    gc: basedrop::Handle,
}

impl Debug for SongEdit<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SongEdit")
            .field("song", &self.song)
            .finish()
    }
}

impl SongEdit<'_> {
    pub fn apply_operation(&mut self, op: SongOperation) -> Result<(), SongOperation> {
        let valid_operation = ValidOperation::new(op, &self.gc, self.song())?;
        self.song.apply_op(valid_operation);
        Ok(())
    }

    pub fn song(&self) -> &Song<true> {
        self.song.read()
    }

    /// Finish the changes and publish them to the live playing song.
    /// Equivalent to std::mem::drop(SongEdit)
    pub fn finish(self) {}
}

#[derive(Debug, Clone, Copy)]
pub struct OutputConfig {
    pub buffer_size: u32,
    pub channel_count: NonZeroU16,
    pub sample_rate: u32,
}

impl From<OutputConfig> for cpal::StreamConfig {
    fn from(value: OutputConfig) -> Self {
        cpal::StreamConfig {
            channels: value.channel_count.into(),
            sample_rate: cpal::SampleRate(value.sample_rate),
            buffer_size: cpal::BufferSize::Fixed(value.buffer_size),
        }
    }
}

impl TryFrom<cpal::StreamConfig> for OutputConfig {
    type Error = ();

    /// fails if BufferSize isn't explicit or zero output channels are specified.
    fn try_from(value: cpal::StreamConfig) -> Result<Self, Self::Error> {
        match value.buffer_size {
            cpal::BufferSize::Default => Err(()),
            cpal::BufferSize::Fixed(size) => Ok(OutputConfig {
                buffer_size: size,
                channel_count: NonZeroU16::try_from(value.channels).map_err(|_| ())?,
                sample_rate: value.sample_rate.0,
            }),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum PlaybackSettings {
    Pattern { idx: usize, should_loop: bool },
    Order { idx: usize, should_loop: bool },
}

impl Default for PlaybackSettings {
    fn default() -> Self {
        Self::Order {
            idx: 0,
            should_loop: false,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum FromWorkerMsg {
    BufferFinished(cpal::OutputStreamTimestamp),
    CurrentPlaybackPosition(PlaybackPosition),
    PlaybackStopped,
}
