use std::{fmt::Debug, mem::ManuallyDrop, num::NonZeroU16};

use basedrop::{Collector, Handle};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use simple_left_right::{WriteGuard, Writer};

use crate::{
    live_audio::{LiveAudio, ToWorkerMsg}, playback::PlaybackPosition, song::song::{Song, SongOperation, ValidOperation}
};

/// blocks via crossbeam_utils
#[inline]
fn send_blocking(writer: &mut rtrb::Producer<ToWorkerMsg>, msg: ToWorkerMsg) {
    let backoff = crossbeam_utils::Backoff::new();
    loop {
        if writer.push(msg).is_ok() {
            return;
        }
        backoff.snooze();
    }
}

pub struct AudioManager {
    song: Writer<Song<true>, ValidOperation>,
    gc: ManuallyDrop<Collector>,
    stream: Option<(cpal::Stream, rtrb::Producer<ToWorkerMsg>)>,
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

    pub fn get_devices() -> cpal::OutputDevices<cpal::Devices> {
        cpal::default_host().output_devices().unwrap()
    }

    pub fn default_device() -> Option<cpal::Device> {
        cpal::default_host().default_output_device()
    }

    /// may block.
    /// 
    /// Spinloops until no more ReadGuard to the old value exists
    pub fn edit_song(&mut self) -> SongEdit<'_> {
        SongEdit {
            song: self.song.lock(),
            gc_handle: self.gc.handle(),
        }
    }

    pub fn get_song(&self) -> &Song<true> {
        self.song.read()
    }

    pub fn collect_garbage(&mut self) {
        self.gc.collect()
    }

    /// If the config specifies more than two channels only the first two will be filled with audio.
    /// The rest gets silence.
    /// audio_msg_config and msg_buffer_size allow you to configure the messages of the audio stream
    /// depending on your application. When the channel is full messages get dropped.
    /// currently panics when there is already a stream. needs better behaviour
    pub fn init_audio(
        &mut self,
        device: cpal::Device,
        config: OutputConfig,
        audio_msg_config: AudioMsgConfig,
        msg_buffer_size: usize,
    ) -> Result<rtrb::Consumer<FromWorkerMsg>, cpal::BuildStreamError> {
        const TO_WORKER_CAPACITY: usize = 5;

        let from_worker = rtrb::RingBuffer::new(msg_buffer_size);
        let to_worker = rtrb::RingBuffer::new(TO_WORKER_CAPACITY);
        let reader = self.song.build_reader().unwrap();

        let audio_worker =
            LiveAudio::new(reader, to_worker.1, audio_msg_config, from_worker.0, config);

        let stream = device.build_output_stream_raw(
            &config.into(),
            cpal::SampleFormat::F32,
            audio_worker.get_generic_callback(),
            |err| println!("{err}"),
            None,
        )?;

        stream.play().unwrap();

        self.stream = Some((stream, to_worker.0));

        Ok(from_worker.1)
    }

    /// pauses the audio thread. only works on some platforms (look at cpal docs)
    pub fn pause_audio(&mut self) {
        if let Some((stream, channel)) = &mut self.stream {
            send_blocking(channel, ToWorkerMsg::StopPlayback);
            stream.pause().unwrap();
        }
    }

    /// resume the audio thread. doesn't start any playback (only on some platforms, see cpal docs for stream.pause())
    pub fn resume_audio(&self) {
        if let Some((stream, _)) = &self.stream {
            stream.play().unwrap();
        }
    }

    pub fn play_note(&mut self, note_event: crate::song::note_event::NoteEvent) {
        if let Some((_, channel)) = &mut self.stream {
            send_blocking(channel, ToWorkerMsg::PlayEvent(note_event));
        }
    }

    pub fn play_song(&mut self, settings: PlaybackSettings) {
        if let Some((_, channel)) = &mut self.stream {
            send_blocking(channel, ToWorkerMsg::Playback(settings));
        }
    }

    pub fn stop_playback(&mut self) {
        if let Some((_, channel)) = &mut self.stream {
            send_blocking(channel, ToWorkerMsg::StopPlayback);
        }
    }

    pub fn deinit_audio(&mut self) {
        if let Some((stream, mut send)) = self.stream.take() {
            send_blocking(&mut send, ToWorkerMsg::StopPlayback);
            drop(stream);
            self.gc.collect();
        }
    }
}

impl Debug for AudioManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AudioManager").field("song", &self.song).field("stream", &self.stream.as_ref().map(|(_, send)| send)).finish()
    }
}

impl Drop for AudioManager {
    /// if this panics the drop implementation isn't right and the Audio Callback isn't cleaned up properly
    fn drop(&mut self) {
        self.deinit_audio();
        let mut gc = unsafe { std::mem::ManuallyDrop::take(&mut self.gc) };
        gc.collect();
        assert!(gc.try_cleanup().is_ok());
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
    gc_handle: Handle,
}

impl Debug for SongEdit<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SongEdit").field("song", &self.song).finish()
    }
}

impl SongEdit<'_> {
    pub fn apply_operation(&mut self, op: SongOperation) -> Result<(), SongOperation> {
        let valid_op = self.song().validate_operation(op, &self.gc_handle)?;
        self.song.apply_op(valid_op);
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


#[derive(Default, Debug, Clone, Copy)]
pub struct AudioMsgConfig {
    pub buffer_finished: bool,
    pub playback_position: bool,
}

#[derive(Debug, Clone, Copy)]
pub enum PlaybackSettings {
    Pattern {
        idx: usize,
        should_loop: bool,
    },
    Order {
        idx: usize,
        should_loop: bool,
    }
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
