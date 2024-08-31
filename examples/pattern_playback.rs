use std::{num::NonZeroU16, time::Duration};

use cpal::{traits::DeviceTrait, Sample};
use impulse_engine::{
    live_audio::{AudioMsgConfig, PlaybackSettings},
    manager::audio_manager::{AudioManager, OutputConfig},
    sample::{SampleData, SampleMetaData},
    song::{
        event_command::NoteCommand,
        note_event::{NoteEvent, VolumeEffect},
        pattern::InPatternPosition,
        song::Song,
    },
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
        ..Default::default()
    };

    let mut song = manager.edit_song();
    song.set_sample(0, meta, sample);
    song.set_note_event(
        0,
        InPatternPosition { row: 0, channel: 0 },
        NoteEvent {
            note: 0,
            sample_instr: 0,
            vol: VolumeEffect::None,
            command: NoteCommand::None,
        },
    );
    song.set_note_event(
        0,
        InPatternPosition { row: 3, channel: 0 },
        NoteEvent {
            note: 0,
            sample_instr: 0,
            vol: VolumeEffect::None,
            command: NoteCommand::None,
        },
    );
    song.set_note_event(
        0,
        InPatternPosition { row: 3, channel: 1 },
        NoteEvent {
            note: 0,
            sample_instr: 0,
            vol: VolumeEffect::None,
            command: NoteCommand::None,
        },
    );
    song.set_order(
        0,
        impulse_engine::file::impulse_format::header::PatternOrder::Number(0),
    );
    song.finish();
    // dbg!(manager.get_song());
    // return;

    let default_device = AudioManager::default_device().unwrap();
    let default_config = default_device.default_output_config().unwrap();
    println!("default config {:?}", default_config);
    println!("device: {:?}", default_device.name());
    let config = OutputConfig {
        buffer_size: 15,
        channel_count: NonZeroU16::new(2).unwrap(),
        sample_rate: default_config.sample_rate().0,
    };

    let mut recv = manager
        .init_audio(
            default_device,
            config,
            AudioMsgConfig {
                playback_position: true,
                ..Default::default()
            },
            20,
        )
        .unwrap();

    manager.play_song(PlaybackSettings {});

    std::thread::sleep(Duration::from_secs(5));
    while let Ok(event) = recv.try_next() {
        println!("{event:?}");
    }
}
