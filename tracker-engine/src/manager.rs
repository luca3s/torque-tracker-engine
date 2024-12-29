use std::{fmt::Debug, mem::ManuallyDrop, num::NonZeroU16, ops::Deref, sync::Arc, time::Duration};

use simple_left_right::{WriteGuard, Writer};

use crate::{
    audio_processing::playback::PlaybackStatus,
    live_audio::LiveAudio,
    project::{
        note_event::NoteEvent,
        song::{Song, SongOperation, ValidOperation},
    },
    sample::{OwnedSample, SharedSample},
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
    AudioInactive,
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

/// This shouldn't get dropped as that will leak the stream.
/// Close it by passing it to AudioManager::close_stream
// this struct prevents a user of the library from closing the Stream by dropping it
pub struct StreamHandle<S>(ManuallyDrop<S>);

impl<S> Deref for StreamHandle<S> {
    type Target = S;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<S> Drop for StreamHandle<S> {
    fn drop(&mut self) {
        eprintln!("StreamHandle dropped. Stream should be closed by passing it to AudioManager::close_stream")
    }
}

pub trait StreamBuilder {
    /// Error when creating the stream
    type CreateErr;
    /// Error while the stream is running
    type StreamErr: Debug;
    /// Stream that is created
    type Stream;
    /// Information given to each buffer callback. Will be made available to the outside code via the Manager
    type BufferInformation: Send + Clone + 'static;
    fn create(
        self,
        data_callback: impl FnMut(&mut [f32], Self::BufferInformation)
            + Send
            + 'static,
        err_callback: impl FnMut(Self::StreamErr) + Send + 'static,
        config: OutputConfig,
    ) -> Result<Self::Stream, Self::CreateErr>;
}

#[cfg(feature = "cpal")]
mod cpal {
    use std::num::NonZeroU16;

    use cpal::traits::StreamTrait;

    use super::{OutputConfig, StreamBuilder};

    impl<Device: cpal::traits::DeviceTrait> StreamBuilder for &Device {
        type CreateErr = cpal::BuildStreamError;
        type StreamErr = cpal::StreamError;
        type Stream = Device::Stream;
        type BufferInformation = cpal::OutputStreamTimestamp;
        fn create(
            self,
            mut data_callback: impl FnMut(
                    &mut [f32],
                    Self::BufferInformation,
                ) + Send
                + 'static,
            err_callback: impl FnMut(Self::StreamErr) + Send + 'static,
            config: OutputConfig,
        ) -> Result<Self::Stream, Self::CreateErr> {
            let stream = self.build_output_stream_raw(
                &config.into(),
                cpal::SampleFormat::F32,
                move |d, i| {
                    let d = d.as_slice_mut().unwrap();
                    data_callback(d, i.timestamp())
                },
                err_callback,
                None,
            );
            stream.inspect(|s|{
                // this error is unlikely to ever happen. We just interacted with the device when starting the stream so interacting with it now should work.
                let _ = s.play().inspect_err(|e| eprintln!("error while starting the stream {}", e));
            })
        }
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
}

/// Communication to and from an active Stream
#[derive(Debug)]
struct ActiveStreamComms<BufferInfo: Send + Clone> {
    buffer_time: Duration,
    send: rtrb::Producer<ToWorkerMsg>,
    status: triple_buffer::Output<(Option<PlaybackStatus>, Option<BufferInfo>)>,
}

#[derive(Debug, Default)]
pub(crate) struct Collector {
    samples: Vec<SharedSample>,
}

impl Collector {
    pub fn add_sample(&mut self, sample: OwnedSample) -> SharedSample {
        let sample = match sample {
            OwnedSample::MonoF32(data) => SharedSample::MonoF32(data.into()),
            OwnedSample::MonoI16(data) => SharedSample::MonoI16(data.into()),
            OwnedSample::MonoI8(data) => SharedSample::MonoI8(data.into()),
            OwnedSample::StereoF32(data) => SharedSample::StereoF32(data.into()),
            OwnedSample::StereoI16(data) => SharedSample::StereoI16(data.into()),
            OwnedSample::StereoI8(data) => SharedSample::StereoI8(data.into()),
        };

        self.samples.push(sample.clone());
        sample
    }

    fn collect(&mut self) {
        self.samples.retain(|s| {
            // only look at strong count as weak pointers are not used
            match s {
                SharedSample::MonoF32(arc) => Arc::strong_count(arc) != 1,
                SharedSample::MonoI16(arc) => Arc::strong_count(arc) != 1,
                SharedSample::MonoI8(arc) => Arc::strong_count(arc) != 1,
                SharedSample::StereoF32(arc) => Arc::strong_count(arc) != 1,
                SharedSample::StereoI16(arc) => Arc::strong_count(arc) != 1,
                SharedSample::StereoI8(arc) => Arc::strong_count(arc) != 1,
            }
        });
    }

    fn sample_count(&self) -> usize {
        self.samples.len()
    }
}

/// You will need to write your own spin loops.
/// For that you can and maybe should use AudioManager::buffer_time.
pub struct AudioManager<BufferInfo: Send + Clone + 'static> {
    song: Writer<Song<true>, ValidOperation>,
    gc: Collector,
    stream_comms: Option<ActiveStreamComms<BufferInfo>>,
}

impl<BufferInfo: Send + Clone + 'static> AudioManager<BufferInfo> {
    pub fn new(song: Song<false>) -> Self {
        let mut gc = Collector::default();
        let left_right = simple_left_right::Writer::new(song.to_gc(&mut gc));

        Self {
            song: left_right,
            gc,
            stream_comms: None,
        }
    }

    pub fn try_edit_song(&mut self) -> Option<SongEdit<'_>> {
        self.song.try_lock().map(|song| SongEdit {
            song,
            gc: &mut self.gc,
        })
    }

    pub fn get_song(&self) -> &Song<true> {
        self.song.read()
    }

    pub fn collect_garbage(&mut self) {
        self.gc.collect();
    }

    pub fn try_msg_worker(&mut self, msg: ToWorkerMsg) -> SendResult {
        if let Some(stream) = &mut self.stream_comms {
            match stream.send.push(msg) {
                Ok(_) => SendResult::Success,
                Err(_) => SendResult::BufferFull,
            }
        } else {
            SendResult::AudioInactive
        }
    }

    /// last playback status sent by the audio worker
    pub fn playback_status(
        &mut self,
    ) -> Option<&(Option<PlaybackStatus>, Option<BufferInfo>)> {
        self.stream_comms.as_mut().map(|s| s.status.read())
    }

    /// Some if a stream is active.
    /// Returns the approximate time it takes to process an audio buffer based on the used settings.
    ///
    /// Useful for implementing spin_loops on collect_garbage or for locking a SongEdit as every time a buffer is finished
    /// garbage could be releases and a lock could be made available
    pub fn buffer_time(&self) -> Option<Duration> {
        self.stream_comms.as_ref().map(|s| s.buffer_time)
    }

    /// If the config specifies more than two channels only the first two will be filled with audio.
    /// The rest gets silence.
    pub fn init_audio<Builder>(
        &mut self,
        create_stream: Builder,
        config: OutputConfig,
    ) -> Result<StreamHandle<Builder::Stream>, Builder::CreateErr>
    where
        Builder: StreamBuilder<BufferInformation = BufferInfo>,
    {
        const TO_WORKER_CAPACITY: usize = 5;

        assert!(self.stream_comms.is_none(), "Stream already active");
        let from_worker = triple_buffer::triple_buffer(&(None, None));
        let to_worker = rtrb::RingBuffer::new(TO_WORKER_CAPACITY);
        let reader = self.song.build_reader().unwrap();

        let audio_worker =
            LiveAudio::<BufferInfo>::new(reader, to_worker.1, from_worker.0, config);

        // let stream = device.build_output_stream_raw(
        //     &config.into(),
        //     cpal::SampleFormat::F32,
        //     audio_worker.get_generic_callback(),
        //     |err| println!("{err}"),
        //     None,
        // )?;
        let stream = create_stream.create(
            audio_worker.get_typed_callback(),
            |err| eprintln!("{err:?}"),
            config,
        )?;
        let buffer_time =
            Duration::from_millis((config.buffer_size * 1000 / config.buffer_size).into());

        self.stream_comms = Some(ActiveStreamComms {
            buffer_time,
            send: to_worker.0,
            status: from_worker.1,
        });

        Ok(StreamHandle(ManuallyDrop::new(stream)))
    }

    pub fn close_stream<S>(&mut self, stream: StreamHandle<S>) {
        self.stream_comms.take().expect("stream wasn't active");
        let mut stream = ManuallyDrop::new(stream);
        unsafe {
            ManuallyDrop::drop(&mut stream.0);
        }
    }
}

impl<BufferInfo: Send + Clone> Drop for AudioManager<BufferInfo> {
    fn drop(&mut self) {
        // try to stop playback if a stream is active
        if let Some(stream) = &mut self.stream_comms {
            eprintln!("AudioManager dropped while audio Stream still active.");
            let msg1 = stream.send.push(ToWorkerMsg::StopLiveNote);
            let msg2 = stream.send.push(ToWorkerMsg::StopPlayback);
            if msg1.is_err() || msg2.is_err() {
                // This happens when the message buffer is full
                eprintln!("Audio playback couldn't be stopped completely");
            } else {
                eprintln!("Audio playback was stopped");
            }
        }
        // try to clean up as much memory as possible
        let mut song = self.try_edit_song().unwrap();
        for i in 0..Song::<true>::MAX_SAMPLES {
            song.apply_operation(SongOperation::RemoveSample(i))
                .unwrap();
        }
        song.finish();
        // lock it once more to ensure that the changes were propagated
        self.try_edit_song().unwrap();
        self.gc.collect();
        // provide a diagnostic when memory is dropped incorrectly
        let count = self.gc.sample_count();
        // no stream is active => we should be able to clean up all the memory.
        if count != 0 && self.stream_comms.is_none() {
            panic!("Collector bug");
        }
        if count != 0 {
            eprintln!("Audio stream wasn't closed before dropping the AudioManager. {} samples were droppen on the audio thread", count)
        }
    }
}

/// the changes made to the song will be made available to the playing live audio as soon as
/// this struct is dropped.
///
/// With this you can load the full song without ever playing a half initialised state
/// when doing mulitple operations this object should be kept as it is
#[derive(Debug)]
pub struct SongEdit<'a> {
    song: WriteGuard<'a, Song<true>, ValidOperation>,
    gc: &'a mut Collector,
}

impl SongEdit<'_> {
    pub fn apply_operation(&mut self, op: SongOperation) -> Result<(), SongOperation> {
        let valid_operation = ValidOperation::new(op, self.gc, self.song.read())?;
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
