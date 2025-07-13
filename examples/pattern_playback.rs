use std::{
    num::{NonZero, NonZeroU16},
    time::Duration,
};

use cpal::traits::{DeviceTrait, HostTrait};
use torque_tracker_engine::{
    file::impulse_format::{header::PatternOrder, sample::VibratoWave},
    manager::{AudioManager, OutputConfig, PlaybackSettings, ToWorkerMsg},
    project::{
        event_command::NoteCommand,
        note_event::{Note, NoteEvent, VolumeEffect},
        pattern::{InPatternPosition, PatternOperation},
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
        sample_rate: NonZero::new(spec.sample_rate).unwrap(),
        base_note: Note::new(64).unwrap(),
        default_volume: 20,
        global_volume: 20,
        default_pan: None,
        vibrato_speed: 0,
        vibrato_depth: 0,
        vibrato_rate: 0,
        vibrato_waveform: VibratoWave::default(),
    };

    let mut song = manager.try_edit_song().unwrap();
    song.apply_operation(SongOperation::SetSample(0, meta, sample))
        .unwrap();
    for i in 0..12 {
        let command = PatternOperation::SetEvent {
            position: InPatternPosition {
                row: i * 2,
                channel: i as u8,
            },
            event: NoteEvent {
                note: Note::new(60 + (i as u8) * 2).unwrap(),
                sample_instr: 0,
                vol: VolumeEffect::None,
                command: NoteCommand::None,
            },
        };
        song.apply_operation(SongOperation::PatternOperation(0, command))
            .unwrap();
    }
    song.apply_operation(SongOperation::SetOrder(0, PatternOrder::Number(0)))
        .unwrap();

    song.finish();

    let host = cpal::default_host();
    let default_device = host.default_output_device().unwrap();
    let default_config = default_device.default_output_config().unwrap();
    println!("default config {:?}", default_config);
    println!("device: {:?}", default_device.name());
    let config = OutputConfig {
        buffer_size: 1024,
        channel_count: NonZeroU16::new(2).unwrap(),
        sample_rate: NonZero::new(default_config.sample_rate().0).unwrap(),
    };

    let mut callback = manager.get_callback::<f32>(config);
    let stream = default_device
        .build_output_stream(
            &default_config.config(),
            move |data, _| callback(data),
            |e| eprintln!("{e:?}"),
            None,
        )
        .unwrap();

    manager
        .try_msg_worker(ToWorkerMsg::Playback(PlaybackSettings::default()))
        .unwrap();

    std::thread::sleep(Duration::from_secs(5));
    println!("{:?}", manager.playback_status());
    drop(stream);
    manager.stream_closed();
}
