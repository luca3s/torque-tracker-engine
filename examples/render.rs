use tracker_engine::{
    audio_processing::playback::PlaybackState,
    file::impulse_format::header::PatternOrder,
    manager::PlaybackSettings,
    project::{
        event_command::NoteCommand,
        note_event::{Note, NoteEvent, VolumeEffect},
        pattern::InPatternPosition,
        song::Song,
    },
    sample::{OwnedSample, Sample, SampleMetaData},
};

fn main() {
    let mut reader =
        hound::WavReader::open("test-files/coin hat with plastic scrunch-JD.wav").unwrap();
    let spec = reader.spec();
    println!("sample specs: {spec:?}");
    assert!(spec.channels == 1);
    let sample_data: Box<[i16]> = reader
        .samples::<i16>()
        // .map(|result| f32::from_sample(result.unwrap()))
        .map(|result| result.unwrap())
        .collect();
    let sample = OwnedSample::MonoI16(sample_data);

    let meta = SampleMetaData {
        sample_rate: spec.sample_rate,
        ..Default::default()
    };

    let mut song: Song<false> = Song::default();
    song.pattern_order[0] = PatternOrder::Number(0);
    song.samples[0] = Some((meta, Sample::<false>::new(sample)));
    song.patterns[0].set_event(
        InPatternPosition { row: 0, channel: 0 },
        NoteEvent {
            note: Note::default(),
            sample_instr: 0,
            vol: VolumeEffect::None,
            command: NoteCommand::None,
        },
    );
    song.patterns[0].set_event(
        InPatternPosition { row: 0, channel: 2 },
        NoteEvent {
            note: Note::default(),
            sample_instr: 0,
            vol: VolumeEffect::None,
            command: NoteCommand::None,
        },
    );

    let mut playback =
        PlaybackState::<false>::new(&song, 44100, PlaybackSettings::default()).unwrap();
    let iter = playback.iter::<0>(&song);
    for _ in iter.take(50) {
        // dbg!(frame);
    }
}
