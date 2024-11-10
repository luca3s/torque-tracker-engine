use std::{
    fmt::Debug, mem::ManuallyDrop, num::NonZeroU16, time::Duration
};

#[cfg(feature = "async")]
use std::ops::ControlFlow;

use basedrop::{Collector, Handle};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use simple_left_right::{WriteGuard, Writer};

use crate::{
    audio_processing::playback::PlaybackPosition,
    live_audio::{LiveAudio, ToWorkerMsg},
    project::song::{Song, SongOperation, ValidOperation},
};

/// If Async is enabled this allows putting the Collector into an Future and communicating with it
enum ManageCollector {
    /// .1: dropped allocs, that will soon be available to free
    Internal(ManuallyDrop<Collector>, usize),
    #[cfg(feature = "async")]
    External(async_channel::Sender<usize>, Handle),
}

impl ManageCollector {
    fn handle(&self) -> Handle {
        match self {
            ManageCollector::Internal(collector, _) => collector.handle(),
            #[cfg(feature = "async")]
            ManageCollector::External(_, handle) => handle.clone(),
        }
    }

    fn collect(&mut self) {
        #[allow(irrefutable_let_patterns)]
        if let Self::Internal(ref mut collector, num) = self {
            while collector.collect_one() {
                *num -= 1;
            }
        }
    }

    fn increase_dropped(&mut self, frees: usize) {
        match self {
            ManageCollector::Internal(_, num) => *num += frees,
            #[cfg(feature = "async")]
            ManageCollector::External(channel, _) => {
                _ = channel.send_blocking(frees);
            },
        }
    }

    #[cfg(feature = "async")]
    async fn async_increase_dropped(&mut self, frees: usize) {
        match self {
            ManageCollector::Internal(_, num) => *num += frees,
            ManageCollector::External(channel, _) => {
                _ = channel.send(frees).await;
            },
        }
    }
}

/// If this is dropped it will leak all current and future sample data.
/// To avoid this put it back inside the AudioManager
/// Alternatively call `collect` until it returns `ControlFlow::Break`. Then dropping doesn't leak
#[cfg(feature = "async")]
pub struct CollectGarbage {
    collector: ManuallyDrop<Collector>,
    channel: Option<async_channel::Receiver<usize>>,
    /// allocs that the AudioManager has removed from the song.
    /// they will be added to the collector queue soon.
    to_be_freed: usize,
}

#[cfg(feature = "async")]
impl CollectGarbage {
    fn new(collector: ManuallyDrop<Collector>, channel: async_channel::Receiver<usize>, to_be_freed: usize) -> Self {
        Self {
            collector,
            channel: Some(channel),
            to_be_freed,
        }
    }

    /// return value indicates if this function needs to be called again to ensure memory cleanup.
    /// can be called in a loop. async sleeps internally
    pub async fn collect(&mut self) -> ControlFlow<()> {
        use futures_lite::future;

        // 44.100 Hz / 256 Frames in a buffer = 172 buffers per second.
        // => 5.8 ms for each buffer
        const SLEEP: Duration = Duration::from_millis(20);

        async fn recv_channel(this: &mut CollectGarbage) {
            if let Some(ref channel) = this.channel {
                match channel.recv().await {
                    Ok(msg) => this.to_be_freed += msg,
                    Err(_) => {
                        this.to_be_freed = this.collector.alloc_count();
                        this.channel = None;
                    }
                }
            }
        }

        async fn sleep(sleep: Duration) {
            async_io::Timer::after(sleep).await;
        }

        while self.collector.collect_one() {
            self.to_be_freed -= 1;
        }

        if self.channel.is_some() {
            if self.to_be_freed == 0 {
                recv_channel(self).await;
            } else {
                future::race(sleep(SLEEP), recv_channel(self)).await;
            }
            ControlFlow::Continue(())
        } else {
            debug_assert_eq!(self.to_be_freed, self.collector.alloc_count());
            if self.to_be_freed == 0 {
                ControlFlow::Break(())
            } else {
                ControlFlow::Continue(())
            }
        }
    }
}

pub struct AudioManager {
    song: Writer<Song<true>, ValidOperation>,
    gc: ManageCollector,
    stream: Option<(cpal::Stream, rtrb::Producer<ToWorkerMsg>)>,
}

impl AudioManager {
    // 44.100 Hz / 256 Frames in a buffer = 172 buffers per second.
    // => 5.8 ms for each buffer
    // should probably be replaced by a computation based on current settings
    const SPIN_SLEEP: Duration = Duration::from_millis(6);

    pub fn new(song: Song<false>) -> Self {
        let gc = std::mem::ManuallyDrop::new(basedrop::Collector::new());
        let left_right = simple_left_right::Writer::new(song.to_gc(&gc.handle()));

        Self {
            song: left_right,
            gc: ManageCollector::Internal(gc, 0),
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
            song: self.song.sleep_lock(Self::SPIN_SLEEP),
            gc: &mut self.gc
        }
    }

    pub fn get_song(&self) -> &Song<true> {
        self.song.read()
    }

    /// if the Gargage Collector was moved out, this does nothing
    pub fn collect_garbage(&mut self) {
        self.gc.collect();
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
        if let Some((stream, _)) = &mut self.stream {
            stream.pause().unwrap();
        }
    }

    /// resume the audio thread. playback is in the same state as before the pause. (only available on some platforms, see cpal docs for stream.pause())
    pub fn resume_audio(&self) {
        if let Some((stream, _)) = &self.stream {
            stream.play().unwrap();
        }
    }

    pub fn send_worker_msg(&mut self, msg: ToWorkerMsg) {
        if let Some((_, channel)) = &mut self.stream {
            let backoff = crossbeam_utils::Backoff::new();
            loop {
                if channel.push(msg).is_ok() {
                    return;
                }

                if backoff.is_completed() {
                    std::thread::sleep(Self::SPIN_SLEEP);
                } else {
                    backoff.snooze();
                }
            }
        }
    }

    pub fn deinit_audio(&mut self) {
        if let Some((stream, _)) = self.stream.take() {
            self.send_worker_msg(ToWorkerMsg::StopPlayback);
            self.send_worker_msg(ToWorkerMsg::StopLiveNote);
            drop(stream);
            #[cfg(not(feature = "async"))]
            self.gc.collect();
        }
    }
}

// needs to be kept in sync with the sync (haha) version of the functions
#[cfg(feature = "async")]
impl AudioManager {
    pub async fn async_send_worker_msg(&mut self, msg: ToWorkerMsg) {
        if let Some((_, channel)) = &mut self.stream {
            let backoff = crossbeam_utils::Backoff::new();
            loop {
                if channel.push(msg).is_ok() {
                    return;
                }

                if backoff.is_completed() {
                    async_io::Timer::after(Self::SPIN_SLEEP).await;
                } else {
                    backoff.snooze();
                }
            }
        }
    }

    pub async fn async_deinit_audio(&mut self) {
        if let Some((stream, _)) = self.stream.take() {
            self.async_send_worker_msg(ToWorkerMsg::StopPlayback).await;
            self.async_send_worker_msg(ToWorkerMsg::StopLiveNote).await;
            drop(stream);
            self.collect_garbage();
        }
    }

    /// Equivalent to ´edit_song´, except that the sleeping in the spin loop is done with async.
    /// This allows using it inside async functin without blocking the runtime
    pub async fn async_edit_song(&mut self) -> SongEdit<'_> {
        let handle = match &self.gc {
            ManageCollector::Internal(collector, _) => collector.handle(),
            ManageCollector::External(_, handle) => handle.clone(),
        };

        SongEdit {
            song: self.song.async_lock(Self::SPIN_SLEEP).await,
            gc: &mut self.gc
        }
    }

    /// returns None if the garbage collector is already external
    pub fn get_garbage_collector(&mut self) -> Option<CollectGarbage> {
        if let ManageCollector::Internal(ref mut collector, num) = self.gc {
            let handle = collector.handle();
            let (sender, recv) = async_channel::unbounded();
            // SAFETY: Value is overwritten in the next line and not being read.
            let collector = unsafe { ManuallyDrop::take(collector) };
            self.gc = ManageCollector::External(sender, handle);

            let external = CollectGarbage::new(ManuallyDrop::new(collector), recv, num);
            Some(external)
        } else {
            None
        }
    }

    /// makes the garbage collector internal again.
    pub fn insert_garbage_collector(&mut self, gc: CollectGarbage) {
        let CollectGarbage {collector, to_be_freed, channel: _} = gc;
        self.gc = ManageCollector::Internal(collector, to_be_freed);
    }
}

impl Debug for AudioManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AudioManager")
            .field("song", &self.song)
            .field("stream", &self.stream.as_ref().map(|(_, send)| send))
            .finish()
    }
}

impl Drop for AudioManager {
    /// if this panics the drop implementation isn't right and the Audio Callback isn't cleaned up properly
    fn drop(&mut self) {
        self.deinit_audio();
        // due to async feature
        #[allow(irrefutable_let_patterns)]
        if let ManageCollector::Internal(ref mut collector, _) = self.gc {
            let mut gc = unsafe { ManuallyDrop::take(collector) };
            gc.collect();
            if gc.try_cleanup().is_err() {
                eprintln!("Sample Data was leaked")
            }
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
    // gc_handle: Handle,
    gc: &'a mut ManageCollector,
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
        let valid_operation = ValidOperation::new(op, &self.gc.handle(), self.song())?;
        if valid_operation.drops_sample(self.song()) {
            self.gc.increase_dropped(1);
        }
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

#[derive(Default, Debug, Clone, Copy)]
pub struct AudioMsgConfig {
    pub buffer_finished: bool,
    pub playback_position: bool,
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
