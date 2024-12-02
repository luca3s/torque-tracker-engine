use std::{num::NonZeroU16, time::Duration};

use cpal::{traits::DeviceTrait, Sample};
use impulse_engine::{
    live_audio::ToWorkerMsg,
    manager::{AudioManager, OutputConfig, PlaybackSettings},
    project::{
        event_command::NoteCommand,
        note_event::{Note, NoteEvent, VolumeEffect},
        pattern::{InPatternPosition, PatternOperation},
        song::{Song, SongOperation},
    },
    sample::{SampleData, SampleMetaData},
};

fn main() {
    let mut manager = AudioManager::new(Song::default());
    let mut reader =
        hound::WavReader::open("test-files/coin hat with plastic scrunch-JD.wav").unwrap();
    let spec = reader.spec();
    println!("sample specs: {spec:?}");
    assert!(spec.channels == 1);
    let sample: SampleData = reader
        .samples::<i16>()
        .map(|result| f32::from_sample(result.unwrap()))
        .collect();
    let meta = SampleMetaData {
        sample_rate: spec.sample_rate,
        base_note: Note::new(64).unwrap(),
        ..Default::default()
    };

    let mut song = manager.edit_song();
    song.apply_operation(SongOperation::SetSample(0, meta, sample))
        .unwrap();
    for i in 0..12 {
        let command = PatternOperation::SetEvent {
            position: InPatternPosition {
                row: i,
                channel: i as u8,
            },
            event: NoteEvent {
                note: Note::new(60 + i as u8).unwrap(),
                sample_instr: 0,
                vol: VolumeEffect::None,
                command: NoteCommand::None,
            },
        };
        song.apply_operation(SongOperation::PatternOperation(0, command))
            .unwrap();
    }
    song.apply_operation(SongOperation::SetOrder(
        0,
        impulse_engine::file::impulse_format::header::PatternOrder::Number(0),
    ))
    .unwrap();

    song.finish();
    dbg!(manager.get_song());

    let default_device = AudioManager::default_device().unwrap();
    let default_config = default_device.default_output_config().unwrap();
    println!("default config {:?}", default_config);
    println!("device: {:?}", default_device.name());
    let config = OutputConfig {
        buffer_size: 15,
        channel_count: NonZeroU16::new(2).unwrap(),
        sample_rate: default_config.sample_rate().0,
    };

    let mut recv = manager.init_audio(default_device, config).unwrap();

    manager.send_worker_msg(ToWorkerMsg::Playback(PlaybackSettings::default()));

    std::thread::sleep(Duration::from_secs(5));
    manager.deinit_audio();
    println!("{:?}", recv.read())
}
