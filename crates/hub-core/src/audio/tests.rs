use std::io::Cursor;
use std::sync::atomic::Ordering;

use super::opus_file::{FRAME, OggOpusWriter, decode_file, decode_ogg_opus};
use super::playback::{Shared, fill};
use super::recorder::waveform;
use super::resample::Resampler;

fn sine(freq: f32, rate: u32, seconds: f32, amp: f32) -> Vec<f32> {
    (0..(rate as f32 * seconds) as usize)
        .map(|i| amp * (2.0 * std::f32::consts::PI * freq * i as f32 / rate as f32).sin())
        .collect()
}

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|s| s * s).sum::<f32>() / x.len() as f32).sqrt()
}

fn crossings(x: &[f32]) -> usize {
    x.windows(2).filter(|w| w[0] <= 0.0 && w[1] > 0.0).count()
}

fn resample(input: &[f32], from: u32, to: u32) -> Vec<f32> {
    let mut r = Resampler::new(from, to);
    let mut out = Vec::new();
    for chunk in input.chunks(441) {
        r.process(chunk, &mut out);
    }
    r.flush(&mut out);
    out
}

fn encode(pcm: &[f32]) -> (Vec<u8>, u32) {
    let mut w = OggOpusWriter::new(Vec::new()).unwrap();
    let pre_skip = w.pre_skip;
    for chunk in pcm.chunks(FRAME) {
        let mut frame = [0f32; FRAME];
        frame[..chunk.len()].copy_from_slice(chunk);
        w.encode_frame(&frame, chunk.len()).unwrap();
    }
    (w.finish().unwrap(), pre_skip)
}

fn pages(bytes: &[u8]) -> Vec<(u8, u64, u32)> {
    let mut out = Vec::new();
    let mut i = 0;
    while i + 27 <= bytes.len() && &bytes[i..i + 4] == b"OggS" {
        let flags = bytes[i + 5];
        let granule = u64::from_le_bytes(bytes[i + 6..i + 14].try_into().unwrap());
        let serial = u32::from_le_bytes(bytes[i + 14..i + 18].try_into().unwrap());
        let segs = bytes[i + 26] as usize;
        let body: usize = bytes[i + 27..i + 27 + segs]
            .iter()
            .map(|b| *b as usize)
            .sum();
        out.push((flags, granule, serial));
        i += 27 + segs + body;
    }
    out
}

#[test]
fn resample_44100_to_48000_keeps_length_frequency_and_energy() {
    let out = resample(&sine(440.0, 44_100, 1.0, 0.5), 44_100, 48_000);
    assert!((out.len() as i64 - 48_000).abs() < 40, "len {}", out.len());
    assert!((crossings(&out) as i64 - 440).abs() <= 2);
    assert!((rms(&out[2000..46_000]) - 0.3535).abs() < 0.01);
}

#[test]
fn resample_downsample_removes_content_above_nyquist() {
    let out = resample(&sine(10_000.0, 48_000, 1.0, 0.5), 48_000, 16_000);
    assert!((out.len() as i64 - 16_000).abs() < 40);
    assert!(rms(&out[500..15_000]) < 0.02);
    let low = resample(&sine(1000.0, 48_000, 1.0, 0.5), 48_000, 16_000);
    assert!((rms(&low[500..15_000]) - 0.3535).abs() < 0.01);
}

#[test]
fn resample_same_rate_is_identity() {
    let input = sine(300.0, 48_000, 0.1, 0.5);
    assert_eq!(resample(&input, 48_000, 48_000), input);
}

#[test]
fn ogg_opus_roundtrip_has_valid_headers_and_exact_duration() {
    let pcm = sine(440.0, 48_000, 2.0, 0.5);
    let (bytes, pre_skip) = encode(&pcm);
    assert_eq!(&bytes[..4], b"OggS");
    assert_eq!(&bytes[28..36], b"OpusHead");
    assert_eq!(bytes[36], 1);
    assert_eq!(bytes[37], 1);
    assert_eq!(u16::from_le_bytes([bytes[38], bytes[39]]) as u32, pre_skip);
    assert_eq!(
        u32::from_le_bytes(bytes[40..44].try_into().unwrap()),
        48_000
    );
    let pages = pages(&bytes);
    assert!(pages.len() > 3);
    assert_eq!(pages[0].0 & 0x02, 0x02);
    assert_eq!(pages[0].1, 0);
    assert!(bytes.windows(8).any(|w| w == b"OpusTags"));
    let last = pages.last().unwrap();
    assert_eq!(last.0 & 0x04, 0x04);
    assert_eq!(last.1, pre_skip as u64 + 96_000);
    assert!(pages.iter().all(|p| p.2 == pages[0].2));
    assert!(pages.windows(2).all(|w| w[0].1 <= w[1].1));
    let decoded = decode_ogg_opus(Cursor::new(bytes)).unwrap();
    assert_eq!(decoded.len(), 96_000);
    let floats: Vec<f32> = decoded.iter().map(|s| *s as f32 / 32768.0).collect();
    let level = rms(&floats[4800..91_000]);
    assert!((level - 0.3535).abs() < 0.05, "rms {level}");
    assert!((crossings(&floats) as i64 - 880).abs() <= 6);
}

#[test]
fn odd_length_audio_is_trimmed_to_exact_samples() {
    let pcm = sine(300.0, 48_000, 0.5, 0.4);
    let pcm = &pcm[..pcm.len() - 123];
    let (bytes, _) = encode(pcm);
    assert_eq!(
        decode_ogg_opus(Cursor::new(bytes)).unwrap().len(),
        pcm.len()
    );
}

#[test]
fn silence_decodes_to_silence() {
    let (bytes, _) = encode(&vec![0.0; 48_000]);
    let decoded = decode_ogg_opus(Cursor::new(bytes)).unwrap();
    assert_eq!(decoded.len(), 48_000);
    assert!(decoded.iter().all(|s| s.abs() < 8));
}

#[test]
fn truncated_file_recovers_what_was_written() {
    let (bytes, _) = encode(&sine(440.0, 48_000, 3.0, 0.5));
    let cut = bytes[..bytes.len() * 6 / 10].to_vec();
    let decoded = decode_ogg_opus(Cursor::new(cut)).unwrap();
    assert!(decoded.len() > 48_000 && decoded.len() < 144_000);
}

#[test]
fn corrupted_files_never_panic() {
    let (mut bytes, _) = encode(&sine(440.0, 48_000, 1.0, 0.5));
    for i in (200..bytes.len()).step_by(37) {
        bytes[i] ^= 0xA5;
    }
    let _ = decode_ogg_opus(Cursor::new(bytes));
    assert!(decode_ogg_opus(Cursor::new(b"OggS garbage".to_vec())).is_err());
    assert!(decode_ogg_opus(Cursor::new(Vec::new())).is_err());
}

#[test]
fn decode_file_handles_ogg_missing_and_unknown_files() {
    let dir = tempfile::tempdir().unwrap();
    let ogg = dir.path().join("a.ogg");
    std::fs::write(&ogg, encode(&sine(440.0, 48_000, 0.5, 0.5)).0).unwrap();
    assert_eq!(decode_file(&ogg).unwrap().len(), 24_000);
    assert!(decode_file(&dir.path().join("nao_existe.ogg")).is_err());
    let junk = dir.path().join("b.mp3");
    std::fs::write(&junk, vec![7u8; 4096]).unwrap();
    assert!(decode_file(&junk).is_err());
}

#[test]
fn wav_files_are_decoded_and_resampled_to_48k() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.wav");
    let rate = 24_000u32;
    let data: Vec<u8> = sine(440.0, rate, 1.0, 0.5)
        .iter()
        .flat_map(|s| ((s * 32767.0) as i16).to_le_bytes())
        .collect();
    let mut wav = Vec::new();
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&rate.to_le_bytes());
    wav.extend_from_slice(&(rate * 2).to_le_bytes());
    wav.extend_from_slice(&2u16.to_le_bytes());
    wav.extend_from_slice(&16u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&(data.len() as u32).to_le_bytes());
    wav.extend_from_slice(&data);
    std::fs::write(&path, wav).unwrap();
    let decoded = decode_file(&path).unwrap();
    assert!(
        (decoded.len() as i64 - 48_000).abs() < 100,
        "len {}",
        decoded.len()
    );
}

#[test]
fn waveform_has_64_points_and_follows_levels() {
    assert_eq!(waveform(&[]), vec![0; 64]);
    assert_eq!(waveform(&vec![0.0; 500]), vec![0; 64]);
    let mut levels = vec![0.3; 100];
    levels.extend(vec![0.0; 100]);
    let w = waveform(&levels);
    assert_eq!(w.len(), 64);
    assert!(w[..32].iter().all(|v| *v == 100));
    assert!(w[32..].iter().all(|v| *v == 0));
    let short = waveform(&[0.1, 0.2]);
    assert_eq!(short.len(), 64);
    assert_eq!(*short.iter().max().unwrap(), 100);
}

fn run_fill(
    shared: &Shared,
    samples: &[i16],
    frames: usize,
    channels: usize,
    rate: f64,
) -> Vec<f32> {
    let mut out = vec![0f32; frames * channels];
    fill(&mut out, channels, rate, samples, shared);
    out
}

#[test]
fn playback_callback_mixes_speed_pause_seek_and_finish() {
    let samples: Vec<i16> = (0..48_000).map(|i| (i % 1000) as i16 * 10).collect();
    let shared = Shared::new(samples.len());
    let out = run_fill(&shared, &samples, 480, 2, 48_000.0);
    assert_eq!(out[0], 0.0);
    assert_eq!(out[2], out[3]);
    assert!((out[2] - 10.0 / 32768.0).abs() < 1e-6);
    assert_eq!(f64::from_bits(shared.pos.load(Ordering::Relaxed)), 480.0);

    shared.speed.store(2.0f32.to_bits(), Ordering::Relaxed);
    run_fill(&shared, &samples, 100, 1, 48_000.0);
    assert_eq!(f64::from_bits(shared.pos.load(Ordering::Relaxed)), 680.0);

    shared.paused.store(true, Ordering::Relaxed);
    let silent = run_fill(&shared, &samples, 100, 1, 48_000.0);
    assert!(silent.iter().all(|v| *v == 0.0));
    assert_eq!(f64::from_bits(shared.pos.load(Ordering::Relaxed)), 680.0);

    shared.paused.store(false, Ordering::Relaxed);
    shared.seek.store(47_990f64.to_bits(), Ordering::Relaxed);
    shared.speed.store(1.0f32.to_bits(), Ordering::Relaxed);
    let tail = run_fill(&shared, &samples, 50, 1, 48_000.0);
    assert!(shared.finished.load(Ordering::Relaxed));
    assert!(tail[20..].iter().all(|v| *v == 0.0));
}

#[test]
fn playback_callback_accounts_for_device_rate() {
    let samples = vec![1000i16; 48_000];
    let shared = Shared::new(samples.len());
    run_fill(&shared, &samples, 441, 1, 44_100.0);
    assert!((f64::from_bits(shared.pos.load(Ordering::Relaxed)) - 480.0).abs() < 1e-6);
}

#[test]
fn public_types_are_send() {
    fn is_send<T: Send>() {}
    is_send::<super::Recorder>();
    is_send::<super::Playback>();
    is_send::<super::AudioError>();
}
