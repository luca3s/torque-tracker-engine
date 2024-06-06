use std::sync::mpsc::{Sender, SyncSender};

use basedrop::{Collector, Shared};
use cpal::BufferSize::Default;

use super::{
    audio_worker::WorkerMsg,
    communication::manager_event::EventCallback,
    constants::MAX_PATTERNS,
    pattern::{Pattern, PatternOperation},
    sample::SampleData,
};

pub enum AudioMsg {}

pub struct AudioManager<T> {
    sender: Sender<AudioMsg>,
    event_cb: Option<T>,
}

impl<T: EventCallback + Send> AudioManager<T> {
    pub fn new(callback: T) -> Self {
        todo!()
    }

    pub fn init_audio(&mut self) {}
}

struct AudioManagerBackend<T: EventCallback + Send> {
    pattern: left_right::WriteHandle<[Pattern; MAX_PATTERNS], PatternOperation>,
    gc: Collector,
    worker_send: Option<SyncSender<WorkerMsg>>,
    // keep the samples available.
    sample_list: Box<[Option<Shared<SampleData>>; 100]>,
    app_callback: T,
}

impl<T: EventCallback + Send> AudioManagerBackend<T> {
    // fn new(callback: T) -> Self {
    //     let (write_handle, read_handle) = left_right::new_from_empty([Pattern::default(); 240]);
    //     Self {
    //         pattern: write_handle,
    //         gc: Collector::new(),
    //         worker_send: None,
    //         sample_list: Box::new([None; 100]),
    //         app_callback: callback,
    //     }
    // }
}
