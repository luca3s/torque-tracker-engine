use std::{num::NonZeroU16, time::Duration};

use cpal::{traits::DeviceTrait, Sample};
use impulse_engine::{
    live_audio::AudioMsgConfig,
    manager::audio_manager::{AudioManager, OutputConfig},
    sample::{SampleData, SampleMetaData},
    song::{self, note_event::{Note, NoteEvent}, song::Song},
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

    manager.edit_song().set_sample(1, meta, sample);

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
                buffer_finished: true,
                ..Default::default()
            },
            20,
        )
        .unwrap();

    let note_event = NoteEvent {
        note: Note::new(90).unwrap(),
        sample_instr: 1,
        vol: song::note_event::VolumeEffect::None,
        command: song::event_command::NoteCommand::None,
    };
    manager.play_note(note_event);
    std::thread::sleep(Duration::from_secs(1));
    manager.play_note(note_event);
    std::thread::sleep(Duration::from_secs(1));
    while let Ok(event) = recv.try_next() {
        println!("{event:?}");
    }
}
