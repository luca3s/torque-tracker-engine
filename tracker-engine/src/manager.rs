use std::{fmt::Debug, num::NonZeroU16, sync::Arc, time::Duration};

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

pub trait OutputStream {
    /// Information that is passed to the callback each time it is called
    type BufferInformation: Send + Clone + 'static;
    type PauseErr;
    type PlayErr;
    fn pause(&mut self) -> Result<(), Self::PauseErr> {
        // what i want
        // Result::<(), !>::Ok(())
        Ok(())
    }
    fn play(&mut self) -> Result<(), Self::PlayErr> {
        Ok(())
    }
}

pub trait StreamBuilder {
    /// Error when creating the stream
    type CreateErr;
    /// Error while the stream is running
    type StreamErr: Debug;
    type Stream: OutputStream;
    fn create(
        self,
        data_callback: impl FnMut(&mut [f32], <Self::Stream as OutputStream>::BufferInformation)
            + Send
            + 'static,
        err_callback: impl FnMut(Self::StreamErr) + Send + 'static,
        config: OutputConfig,
    ) -> Result<Self::Stream, Self::CreateErr>;
}

#[cfg(feature = "cpal")]
mod cpal {
    use std::num::NonZeroU16;

    use super::{OutputConfig, OutputStream, StreamBuilder};


    impl<Stream: cpal::traits::StreamTrait> OutputStream for Stream {
        type BufferInformation = cpal::OutputStreamTimestamp;
    
        type PauseErr = cpal::PauseStreamError;
    
        type PlayErr = cpal::PlayStreamError;
    
        fn pause(&mut self) -> Result<(), Self::PauseErr> {
            <Self as cpal::traits::StreamTrait>::pause(self)
        }
    
        fn play(&mut self) -> Result<(), Self::PlayErr> {
            <Self as cpal::traits::StreamTrait>::play(self)
        }
    }

    impl<Device: cpal::traits::DeviceTrait> StreamBuilder for &Device {
        type CreateErr = cpal::BuildStreamError;
        type StreamErr = cpal::StreamError;
        type Stream = Device::Stream;
        fn create(
            self,
            mut data_callback: impl FnMut(&mut [f32], <<Self as StreamBuilder>::Stream as OutputStream>::BufferInformation)
                + Send
                + 'static,
            err_callback: impl FnMut(Self::StreamErr) + Send + 'static,
            config: OutputConfig,
        ) -> Result<Self::Stream, Self::CreateErr> {
            self.build_output_stream_raw(
                &config.into(),
                cpal::SampleFormat::F32,
                move |d, i| {
                    let d = d.as_slice_mut().unwrap();
                    data_callback(d, i.timestamp())
                },
                err_callback,
                None,
            )
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

struct ActiveStream<S: OutputStream> {
    stream: S,
    buffer_time: Duration,
    send: rtrb::Producer<ToWorkerMsg>,
    status: triple_buffer::Output<(Option<PlaybackStatus>, Option<S::BufferInformation>)>,
}

impl<S: OutputStream> Debug for ActiveStream<S> 
where 
    S::BufferInformation: Debug
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ActiveStream")
            .field("buffer_time", &self.buffer_time)
            .field("send", &self.send)
            .field("status", &self.status)
            .finish()
    }
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
        // debug_assert!(Arc::strong_count(sample) == 2);
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
pub struct AudioManager<S: OutputStream> {
    song: Writer<Song<true>, ValidOperation>,
    gc: Collector,
    stream: Option<ActiveStream<S>>,
}

impl<S: OutputStream> AudioManager<S> {
    pub fn new(song: Song<false>) -> Self {
        let mut gc = Collector::default();
        let left_right = simple_left_right::Writer::new(song.to_gc(&mut gc));

        Self {
            song: left_right,
            gc,
            stream: None,
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
    pub fn playback_status(&mut self) -> Option<&(Option<PlaybackStatus>, Option<S::BufferInformation>)> {
        self.stream.as_mut().map(|s| s.status.read())
    }

    /// Some if a stream is active.
    /// Returns the approximate time it takes to process an audio buffer based on the used settings.
    ///
    /// Useful for implementing spin_loops on collect_garbage or for locking a SongEdit as every time a buffer is finished
    /// garbage could be releases and a lock could be made available
    pub fn buffer_time(&self) -> Option<Duration> {
        self.stream.as_ref().map(|s| s.buffer_time)
    }

    /// pauses the audio. only works on some platforms (look at cpal docs)
    pub fn pause_audio(&mut self) -> Option<Result<(), S::PauseErr>> {
        self.stream.as_mut().map(|s| s.stream.pause())
        // if let Some(stream) = &mut self.stream {
        //     stream.stream.pause();
        // }
    }

    /// resume the audio. playback is in the same state as before the pause. (only available on some platforms, see cpal docs for stream.pause())
    pub fn resume_audio(&mut self) -> Option<Result<(), <S as OutputStream>::PlayErr>> {
        self.stream.as_mut().map(|s| s.stream.play())
    }

    /// If the config specifies more than two channels only the first two will be filled with audio.
    /// The rest gets silence.
    pub fn init_audio<Builder>(
        &mut self,
        create_stream: Builder,
        config: OutputConfig,
    ) -> Result<(), Builder::CreateErr> 
    where
        Builder: StreamBuilder<Stream = S>
    {
        const TO_WORKER_CAPACITY: usize = 5;

        let from_worker = triple_buffer::triple_buffer(&(None, None));
        let to_worker = rtrb::RingBuffer::new(TO_WORKER_CAPACITY);
        let reader = self.song.build_reader().unwrap();

        let audio_worker = LiveAudio::<S::BufferInformation>::new(reader, to_worker.1, from_worker.0, config);

        // let stream = device.build_output_stream_raw(
        //     &config.into(),
        //     cpal::SampleFormat::F32,
        //     audio_worker.get_generic_callback(),
        //     |err| println!("{err}"),
        //     None,
        // )?;
        let stream = create_stream.create(audio_worker.get_typed_callback(), |err| eprintln!("{err:?}"), config)?;
        let buffer_time =
            Duration::from_millis((config.buffer_size * 1000 / config.buffer_size).into());

        // stream.play();

        self.stream = Some(ActiveStream {
            stream,
            buffer_time,
            send: to_worker.0,
            status: from_worker.1,
        });

        Ok(())
    }
}

impl<S: OutputStream> Drop for AudioManager<S> {
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
        self.gc.collect();
        // provide a diagnostic when memory is leaked
        let count = self.gc.sample_count();
        if count != 0 {
            eprintln!("Audio thread cleanup didn't run before dropping the AudioManager. {} samples were leaked", count)
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
