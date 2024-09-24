use std::{mem::ManuallyDrop, num::NonZeroU16};

use basedrop::{Collector, Handle, Shared};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use simple_left_right::{WriteGuard, Writer};

use crate::{
    channel::Pan,
    file::impulse_format::header::PatternOrder,
    live_audio::{AudioMsgConfig, FromWorkerMsg, LiveAudio, PlaybackSettings, ToWorkerMsg},
    sample::{SampleData, SampleMetaData},
    song::{
        pattern::PatternOperation,
        song::{Song, SongOperation},
    },
};

pub struct AudioManager {
    song: Writer<Song<true>, SongOperation>,
    gc: ManuallyDrop<Collector>,
    stream: Option<(cpal::Stream, std::sync::mpsc::Sender<ToWorkerMsg>)>,
}

impl AudioManager {
    pub fn new(song: Song<false>) -> Self {
        let gc = std::mem::ManuallyDrop::new(basedrop::Collector::new());
        let left_right: simple_left_right::Writer<Song<true>, SongOperation> = simple_left_right::Writer::new(song.to_gc(&gc.handle()));

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
    pub fn edit_song(&mut self) -> SongEdit {
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
    ) -> Result<futures::channel::mpsc::Receiver<FromWorkerMsg>, cpal::BuildStreamError> {
        let from_worker = futures::channel::mpsc::channel(msg_buffer_size);
        let to_worker = std::sync::mpsc::channel();
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
    pub fn pause_audio(&self) {
        if let Some((stream, channel)) = &self.stream {
            channel.send(ToWorkerMsg::StopPlayback).unwrap();
            stream.pause().unwrap();
        }
    }

    /// resume the audio thread. doesn't start any playback (only on some platforms, see cpal docs for stream.pause())
    pub fn resume_audio(&self) {
        if let Some((stream, _)) = &self.stream {
            stream.play().unwrap();
        }
    }

    pub fn play_note(&self, note_event: crate::song::note_event::NoteEvent) {
        if let Some((_, channel)) = &self.stream {
            channel.send(ToWorkerMsg::PlayEvent(note_event)).unwrap();
        }
    }

    pub fn play_song(&self, settings: PlaybackSettings) {
        if let Some((_, channel)) = &self.stream {
            channel.send(ToWorkerMsg::Playback(settings)).unwrap();
        }
    }

    pub fn stop_playback(&self) {
        if let Some((_, channel)) = &self.stream {
            channel.send(ToWorkerMsg::StopPlayback).unwrap();
        }
    }

    pub fn deinit_audio(&mut self) {
        if let Some((stream, send)) = self.stream.take() {
            send.send(ToWorkerMsg::StopPlayback).unwrap();
            drop(stream);
            self.gc.collect();
        }
    }
}

impl Drop for AudioManager {
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
    song: WriteGuard<'a, Song<true>, SongOperation>,
    gc_handle: Handle,
}

impl SongEdit<'_> {
    pub fn set_sample(&mut self, num: usize, meta: SampleMetaData, data: SampleData) {
        assert!(num < Song::<false>::MAX_SAMPLES);
        let op = SongOperation::SetSample(num, meta, Shared::new(&self.gc_handle, data));
        self.song.apply_op(op);
    }

    pub fn set_volume(&mut self, channel: usize, volume: u8) {
        assert!(channel < Song::<false>::MAX_CHANNELS);
        let op = SongOperation::SetVolume(channel, volume);
        self.song.apply_op(op);
    }

    pub fn set_pan(&mut self, channel: usize, pan: Pan) {
        assert!(channel < Song::<false>::MAX_CHANNELS);
        let op = SongOperation::SetPan(channel, pan);
        self.song.apply_op(op);
    }

    pub fn pattern_operation(&mut self, pattern: usize, op: PatternOperation) {
        assert!(pattern < Song::<false>::MAX_PATTERNS);
        assert!(self.song.read().patterns[pattern].operation_is_valid(&op));
        self.song.apply_op(SongOperation::PatternOperation(pattern, op));
    }

    pub fn set_order(&mut self, idx: usize, order: PatternOrder) {
        assert!(idx < Song::<false>::MAX_ORDERS);
        let op = SongOperation::SetOrder(idx, order);
        self.song.apply_op(op);
    }

    pub fn song(&self) -> &Song<true> {
        self.song.read()
    }

    /// Finish the changes and publish them to the live playing song
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
