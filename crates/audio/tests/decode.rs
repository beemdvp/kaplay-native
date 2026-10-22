use kaplay_native_audio::{decode, AudioBuffer};

fn energy(buffer: &AudioBuffer) -> f32 {
    let samples = buffer.samples();
    if samples.is_empty() {
        return 0.0;
    }
    (samples.iter().map(|sample| sample * sample).sum::<f32>() / samples.len() as f32).sqrt()
}

fn assert_tone(buffer: &AudioBuffer, rate: u32) {
    assert_eq!(buffer.channels, 2);
    assert_eq!(buffer.sample_rate, rate);
    assert!(
        (buffer.duration() - 0.25).abs() < 0.05,
        "{}",
        buffer.duration()
    );
    assert!(energy(buffer) > 0.05, "{}", energy(buffer));
}

#[test]
fn decodes_wav() {
    let buffer = decode(include_bytes!("fixtures/tone.wav")).unwrap();
    assert_tone(&buffer, 44100);
}

#[test]
fn decodes_vorbis() {
    let buffer = decode(include_bytes!("fixtures/tone-vorbis.ogg")).unwrap();
    assert_tone(&buffer, 44100);
}

#[test]
fn decodes_mp3() {
    let buffer = decode(include_bytes!("fixtures/tone.mp3")).unwrap();
    assert_tone(&buffer, 44100);
}

#[test]
fn decodes_opus() {
    let buffer = decode(include_bytes!("fixtures/tone-opus.opus")).unwrap();
    assert_tone(&buffer, 48000);
}

#[test]
fn rejects_unknown_data() {
    assert!(decode(b"not audio at all").is_err());
}

#[test]
fn uploads_and_reads_channels() {
    let audio = kaplay_native_audio::Audio::silent();
    let samples = vec![0.5f32, -0.5, 0.25, -0.25];
    let id = audio.upload_buffer(48000, 2, samples);
    assert_eq!(audio.buffer_count(), 1);
    assert_eq!(audio.buffer_info(id), Some((48000, 2, 2)));
    let mut channel = [0.0f32; 4];
    assert_eq!(audio.read_channel(id, 0, &mut channel), 2);
    assert_eq!(&channel[..2], &[0.5, 0.25]);
    assert_eq!(audio.read_channel(id, 1, &mut channel), 2);
    assert_eq!(&channel[..2], &[-0.5, -0.25]);
    audio.free_buffer(id);
    assert_eq!(audio.buffer_count(), 0);
    assert_eq!(audio.buffer_info(id), None);
}
