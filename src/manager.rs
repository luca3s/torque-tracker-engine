use std::{fmt::Debug, num::NonZeroU16, time::Duration};

use simple_left_right::{WriteGuard, Writer};

use crate::{
    audio_processing::playback::PlaybackStatus,
    live_audio::LiveAudio,
    project::{
        note_event::NoteEvent,
        song::{Song, SongOperation, ValidOperation},
    },
    sample::Sample,
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

/// Communication to and from an active Stream
#[derive(Debug)]
struct ActiveStreamComms {
    buffer_time: Duration,
    send: rtrb::Producer<ToWorkerMsg>,
    status: triple_buffer::Output<Option<PlaybackStatus>>,
}

#[derive(Debug, Default)]
pub(crate) struct Collector {
    samples: Vec<Sample>,
}

impl Collector {
    pub fn add_sample(&mut self, sample: Sample) {
        self.samples.push(sample);
    }

    fn collect(&mut self) {
        self.samples.retain(|s| {
            // only look at strong count as weak pointers are not used
            s.strongcount() != 1
        });
    }
}

/// You will need to write your own spin loops.
/// For that you can and maybe should use AudioManager::buffer_time.
///
/// The Stream API is not "Rusty" and not ergonimic to use, but Stream are often not Send, while the Manager is
/// suited well for being in a Global Mutex. This is why the Stream can't live inside the Manager. If you can
/// think of a better API i would love to replace this.
pub struct AudioManager {
    song: Writer<Song, ValidOperation>,
    gc: Collector,
    stream_comms: Option<ActiveStreamComms>,
}

impl AudioManager {
    pub fn new(song: Song) -> Self {
        let mut gc = Collector::default();
        for (_, sample) in song.samples.iter().flatten() {
            gc.add_sample(sample.clone());
        }
        let left_right = simple_left_right::Writer::new(song);

        Self {
            song: left_right,
            gc,
            stream_comms: None,
        }
    }

    /// If this returns None, waiting buffer_time should (weird threading issues aside) always be enough time
    /// and it should return Some after that.
    pub fn try_edit_song(&mut self) -> Option<SongEdit<'_>> {
        self.song.try_lock().map(|song| SongEdit {
            song,
            gc: &mut self.gc,
        })
    }

    pub fn get_song(&self) -> &Song {
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
    pub fn playback_status(&mut self) -> Option<&Option<PlaybackStatus>> {
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
    ///
    /// The callback in for example Cpal provides an additional arguement, where a timestamp is give.
    /// That should ba handled by wrapping this function in another callback, where this argument could
    /// then be ignored or send somewhere for processing. This Sending needs to happen wait-free!! There are
    /// a couple of libaries that can do this, i would recommend triple_buffer.
    ///
    /// The OutputConfig has to match the config of the AudioStream that will call this. If for example the
    /// buffer size is different Panics will occur.
    ///
    /// In my testing i noticed that when using Cpal with non-standard buffer sizes Cpal would just give
    /// another buffer size. This will also lead to panics.
    ///
    /// The stream has to closed before dropping the Manager and the manager has to be notified by calling stream_closed.
    pub fn get_callback<Sample: dasp::sample::Sample + dasp::sample::FromSample<f32>>(
        &mut self,
        config: OutputConfig,
    ) -> impl FnMut(&mut [Sample]) {
        const TO_WORKER_CAPACITY: usize = 5;

        assert!(self.stream_comms.is_none(), "Stream already active");
        let from_worker = triple_buffer::triple_buffer(&None);
        let to_worker = rtrb::RingBuffer::new(TO_WORKER_CAPACITY);
        let reader = self.song.build_reader().unwrap();

        let audio_worker = LiveAudio::new(reader, to_worker.1, from_worker.0, config);
        let buffer_time =
            Duration::from_millis((config.buffer_size * 1000 / config.buffer_size).into());

        self.stream_comms = Some(ActiveStreamComms {
            buffer_time,
            send: to_worker.0,
            status: from_worker.1,
        });

        audio_worker.get_typed_callback()
    }

    /// When closing the Stream this method should be called.
    pub fn stream_closed(&mut self) {
        self.stream_comms = None
    }
}

impl Drop for AudioManager {
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
    }
}

/// the changes made to the song will be made available to the playing live audio as soon as
/// this struct is dropped.
///
/// With this you can load the full song without ever playing a half initialised state
/// when doing mulitple operations this object should be kept as it is
#[derive(Debug)]
pub struct SongEdit<'a> {
    song: WriteGuard<'a, Song, ValidOperation>,
    gc: &'a mut Collector,
}

impl SongEdit<'_> {
    pub fn apply_operation(&mut self, op: SongOperation) -> Result<(), SongOperation> {
        let valid_operation = ValidOperation::new(op, self.gc, self.song.read())?;
        self.song.apply_op(valid_operation);
        Ok(())
    }

    pub fn song(&self) -> &Song {
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
