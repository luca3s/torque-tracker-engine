use std::{num::NonZeroU16, time::Duration};

use cpal::traits::{DeviceTrait, HostTrait};
use torque_tracker_engine::{
    manager::{AudioManager, OutputConfig, ToWorkerMsg},
    project::{
        event_command::NoteCommand,
        note_event::{Note, NoteEvent, VolumeEffect},
        song::{Song, SongOperation},
    },
    sample::{Sample, SampleMetaData},
};

fn main() {
    let mut manager = AudioManager::new(Song::default());
    let mut reader = hound::WavReader::open("test-files/770_Hz_Tone.wav").unwrap();
    let spec = reader.spec();
    println!("sample specs: {spec:?}");
    assert!(spec.channels == 1);
    let sample_data = reader
        .samples::<i16>()
        .map(|result| <f32 as dasp::Sample>::from_sample(result.unwrap()));
    let sample = Sample::new_mono(sample_data);
    let meta = SampleMetaData {
        sample_rate: spec.sample_rate,
        default_volume: 150,
        ..Default::default()
    };

    manager
        .try_edit_song()
        .unwrap()
        .apply_operation(SongOperation::SetSample(1, meta, sample))
        .unwrap();

    let host = cpal::default_host();
    let default_device = host.default_output_device().unwrap();
    let default_config = default_device.default_output_config().unwrap();
    println!("default config {:?}", default_config);
    println!("device: {:?}", default_device.name());
    let config = OutputConfig {
        buffer_size: 2048,
        channel_count: NonZeroU16::new(2).unwrap(),
        sample_rate: default_config.sample_rate().0,
    };

    let mut audio_callback = manager.get_callback::<f32>(config);
    let stream = default_device
        .build_output_stream(
            &default_config.config(),
            move |data, _| audio_callback(data),
            |e| eprintln!("{e:?}"),
            None,
        )
        .unwrap();

    let note_event = NoteEvent {
        note: Note::new(90).unwrap(),
        sample_instr: 1,
        vol: VolumeEffect::None,
        command: NoteCommand::None,
    };
    manager
        .try_msg_worker(ToWorkerMsg::PlayEvent(note_event))
        .unwrap();
    std::thread::sleep(Duration::from_secs(1));
    manager
        .try_msg_worker(ToWorkerMsg::PlayEvent(note_event))
        .unwrap();
    std::thread::sleep(Duration::from_secs(3));
    drop(stream);
    manager.stream_closed();
}
