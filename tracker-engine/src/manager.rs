use std::{
    fmt::Debug,
    mem::{transmute, ManuallyDrop},
    num::{NonZero, NonZeroU16},
    time::Duration,
};

use basedrop::{Collector, Handle};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use simple_left_right::{WriteGuard, Writer};

use crate::{
    audio_processing::playback::PlaybackPosition,
    live_audio::{LiveAudio, ToWorkerMsg},
    project::song::{Song, SongOperation, ValidOperation},
};

/// blocks via crossbeam_utils
#[inline]
fn send_blocking(writer: &mut rtrb::Producer<ToWorkerMsg>, msg: ToWorkerMsg, sleep: Duration) {
    let backoff = crossbeam_utils::Backoff::new();
    loop {
        if writer.push(msg).is_ok() {
            return;
        }

        if backoff.is_completed() {
            std::thread::sleep(sleep);
        } else {
            backoff.snooze();
        }
    }
}

#[inline]
#[cfg(feature = "async")]
async fn send_async(writer: &mut rtrb::Producer<ToWorkerMsg>, msg: ToWorkerMsg, sleep: Duration) {
    let backoff = crossbeam_utils::Backoff::new();
    loop {
        if writer.push(msg).is_ok() {
            return;
        }

        if backoff.is_completed() {
            async_io::Timer::after(sleep).await;
        } else {
            backoff.snooze();
        }
    }
}

// #[cfg(feature = "async")]
struct AllocsFreed(usize);

/// If Async is enabled this allows putting the Collector into an Future and communicating with it
enum ManageCollector {
    Internal(ManuallyDrop<Collector>),
    #[cfg(feature = "async")]
    External(async_channel::Sender<AllocsFreed>, Handle),
}

#[cfg(not(feature = "async"))]
impl ManageCollector {
    fn handle(&self) -> Handle {
        let Self::Internal(ref collector) = self;
        collector.handle()
    }

    fn collect(&mut self) {
        let Self::Internal(ref mut collector) = self;
        collector.collect();
    }
}

// #[cfg(feature = "async")]
enum CollectorFutState {
    ToCollect(NonZero<usize>),
}

/// If this is dropped it will leak all current and future sample data.
// #[cfg(feature = "async")]
pub struct CollectorFut {
    collector: Collector,
    channel: async_channel::Receiver<AllocsFreed>,
    recv: Option<async_channel::Recv<'static, AllocsFreed>>,
    /// allocs that the AudioManager has removed from the song.
    /// they will be added to the collector queue soon.
    to_be_freed: usize,
    /// false if the AudioManager was dropped. Set to false after getting a disconnect Err from the channel once
    more_allocs_possible: bool,
    /// needed for recv, as it has a ref to channel
    _pin: std::marker::PhantomPinned,
}

// #[cfg(feature = "async")]
impl CollectorFut {
    fn new(collector: Collector, channel: async_channel::Receiver<AllocsFreed>) -> Self {
        Self {
            collector,
            channel,
            recv: None,
            to_be_freed: 0,
            // it was just created, so the manager is alive
            more_allocs_possible: true,
            _pin: std::marker::PhantomPinned,
        }
    }
}

// #[cfg(feature = "async")]
impl std::future::Future for CollectorFut {
    type Output = ();

    fn poll(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        // 44.100 Hz / 256 Frames in a buffer = 172 buffers per second.
        // => 5.8 ms for each buffer
        const SLEEP: Duration = Duration::from_millis(6);

        use std::task::Poll;
        use std::pin::Pin;
        use futures_lite::future::FutureExt;

        if self.as_ref().more_allocs_possible {
            if self.as_ref().recv.is_none() {
                // set self.recv
                let recv = self.channel.recv();
                // remove the lifetime.
                // SAFETY: lifetime is bound to self.channel and the object is Pin and PhantomPinned
                let recv = unsafe {
                    transmute::<async_channel::Recv<'_, AllocsFreed>, async_channel::Recv<'static, AllocsFreed>>(recv)
                };
                // create a recv future
                let mut this_recv = unsafe { self.as_mut().map_unchecked_mut(|this| &mut this.recv) };

                this_recv.set(Some(recv));
            }

            // recv is Some now
            // take a Pin<&mut> to that Some
            let recv = unsafe { self.as_mut().map_unchecked_mut(|this| this.recv.as_mut().unwrap()) };
            match recv.poll(cx) {
                Poll::Ready(result) => {
                    // poll is ready => Drop the Future
                    recv.set(None);
                    // get mut ref for Unpin editing
                    let this = unsafe { Pin::into_inner_unchecked(self.as_mut()) };
                    match result {
                        Ok(allocs) => this.to_be_freed += allocs.0,
                        Err(_) => {
                            this.more_allocs_possible = false;
                            this.to_be_freed = this.collector.alloc_count();
                        }
                    }
                }
                Poll::Pending => (),
            }
        } else {
            // nothing to do as no new info will come in
            debug_assert!(self.collector.alloc_count() == self.to_be_freed);
        }



        Poll::Pending
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
            gc: ManageCollector::Internal(gc),
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
        #[cfg(feature = "async")]
        let handle = match &self.gc {
            ManageCollector::Internal(collector) => collector.handle(),
            ManageCollector::External(_, handle) => handle.clone(),
        };

        #[cfg(not(feature = "async"))]
        let handle = self.gc.handle();

        SongEdit {
            song: self.song.lock(Self::SPIN_SLEEP),
            gc_handle: handle,
        }
    }

    pub fn get_song(&self) -> &Song<true> {
        self.song.read()
    }

    pub fn collect_garbage(&mut self) {
        // expect it due to async feature cfg
        #[allow(irrefutable_let_patterns)]
        if let ManageCollector::Internal(collector) = &mut self.gc {
            collector.collect();
        }
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
            send_blocking(channel, msg, Self::SPIN_SLEEP);
        }
    }

    pub fn deinit_audio(&mut self) {
        if let Some((stream, mut send)) = self.stream.take() {
            send_blocking(&mut send, ToWorkerMsg::StopPlayback, Self::SPIN_SLEEP);
            send_blocking(&mut send, ToWorkerMsg::StopLiveNote, Self::SPIN_SLEEP);
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
            send_async(channel, msg, Self::SPIN_SLEEP).await;
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
            ManageCollector::Internal(collector) => collector.handle(),
            ManageCollector::External(_, handle) => handle.clone(),
        };

        SongEdit {
            song: self.song.async_lock(Self::SPIN_SLEEP).await,
            gc_handle: handle,
        }
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
        if let ManageCollector::Internal(ref mut collector) = self.gc {
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
    gc_handle: Handle,
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
        self.song
            .apply_op(ValidOperation::new(op, &self.gc_handle, self.song())?);
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
