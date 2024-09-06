mod audio_processing;
mod channel;
pub mod file;
pub mod live_audio;
pub mod manager;
pub mod playback;
pub mod sample;
pub mod song;

#[cfg(test)]
mod tests {
    use std::{num::NonZeroU16, time::Duration};

    use cpal::{traits::DeviceTrait, Sample};
    use live_audio::AudioMsgConfig;
    use manager::audio_manager::{AudioManager, OutputConfig};
    use sample::{SampleData, SampleMetaData};
    use song::{note_event::{Note, NoteEvent}, song::Song};

    use super::*;

    #[test]
    fn live_note() {
        let mut manager = AudioManager::new(Song::default());
        let mut reader = hound::WavReader::open("test-files/Metal-ScuffedOpen24.wav").unwrap();
        let spec = reader.spec();
        println!("sample specs: {spec:?}");
        assert!(spec.channels == 1);
        let sample: SampleData = reader
            .samples::<i32>()
            .map(|result| f32::from_sample(result.unwrap()))
            .collect();
        let meta = SampleMetaData {
            sample_rate: spec.sample_rate,
            ..Default::default()
        };

        manager.edit_song().set_sample(1, meta, sample);

        let default_device = AudioManager::default_device().unwrap();
        println!(
            "default config {:?}",
            default_device.default_output_config().unwrap()
        );
        println!("device: {:?}", default_device.name());
        let config = OutputConfig {
            buffer_size: 512,
            channel_count: NonZeroU16::new(2).unwrap(),
            sample_rate: 48_000,
        };

        let mut recv = manager
            .init_audio(default_device, config, AudioMsgConfig::default(), 20)
            .unwrap();

        let note_event = NoteEvent {
            note: Note::new(0).unwrap(),
            sample_instr: 1,
            vol: song::note_event::VolumeEffect::None,
            command: song::event_command::NoteCommand::None,
        };
        manager.play_note(note_event);
        std::thread::sleep(Duration::from_secs(3));
        manager.play_note(note_event);
        std::thread::sleep(Duration::from_secs(3));
        while let Ok(event) = recv.try_next() {
            println!("{event:?}");
        }
    }
}
