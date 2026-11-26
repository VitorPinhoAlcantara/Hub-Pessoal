use std::fs::{self, File};
use std::io::BufWriter;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Device, FromSample, Sample, SampleFormat, SizedSample, StreamConfig};

use super::opus_file::{FRAME, OggOpusWriter};
use super::resample::{RATE, Resampler};
use super::{AudioError, VoiceNote, map_cpal};

const MAX_SAMPLES: u64 = 30 * 60 * RATE as u64;
const WAVEFORM_POINTS: usize = 64;

struct Shared {
    samples: AtomicU64,
    stop: AtomicBool,
    error: Mutex<Option<String>>,
}

struct Outcome {
    samples: u64,
    levels: Vec<f32>,
}

pub struct Recorder {
    path: PathBuf,
    shared: Arc<Shared>,
    handle: Option<JoinHandle<Result<Outcome, AudioError>>>,
}

impl Recorder {
    pub fn start(path: PathBuf) -> Result<Recorder, AudioError> {
        let shared = Arc::new(Shared {
            samples: AtomicU64::new(0),
            stop: AtomicBool::new(false),
            error: Mutex::new(None),
        });
        let (ready_tx, ready_rx) = mpsc::channel();
        let thread_shared = shared.clone();
        let thread_path = path.clone();
        let handle = std::thread::Builder::new()
            .name("hub-audio-rec".into())
            .spawn(move || run(thread_path, thread_shared, ready_tx))
            .map_err(|e| AudioError::Device(e.to_string()))?;
        match ready_rx.recv() {
            Ok(Ok(())) => Ok(Recorder {
                path,
                shared,
                handle: Some(handle),
            }),
            Ok(Err(e)) => {
                let _ = handle.join();
                let _ = fs::remove_file(&path);
                Err(e)
            }
            Err(_) => {
                let _ = handle.join();
                let _ = fs::remove_file(&path);
                Err(AudioError::Device("a gravação não pôde iniciar".into()))
            }
        }
    }

    pub fn elapsed(&self) -> Duration {
        Duration::from_secs_f64(self.shared.samples.load(Ordering::Relaxed) as f64 / RATE as f64)
    }

    pub fn error(&self) -> Option<String> {
        self.shared.error.lock().ok().and_then(|e| e.clone())
    }

    pub fn finish(mut self) -> Result<VoiceNote, AudioError> {
        let outcome = self.join()?;
        if outcome.samples == 0 {
            let _ = fs::remove_file(&self.path);
            return Err(AudioError::Empty);
        }
        Ok(VoiceNote {
            path: self.path.clone(),
            seconds: ((outcome.samples as f64 / RATE as f64).round() as u32).max(1),
            waveform: waveform(&outcome.levels),
        })
    }

    pub fn cancel(mut self) {
        let _ = self.join();
        let _ = fs::remove_file(&self.path);
    }

    fn join(&mut self) -> Result<Outcome, AudioError> {
        self.shared.stop.store(true, Ordering::Relaxed);
        match self.handle.take() {
            Some(h) => h
                .join()
                .unwrap_or_else(|_| Err(AudioError::Device("falha interna na gravação".into()))),
            None => Err(AudioError::Empty),
        }
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        if self.handle.is_some() {
            let _ = self.join();
            let _ = fs::remove_file(&self.path);
        }
    }
}

pub fn waveform(levels: &[f32]) -> Vec<u8> {
    if levels.is_empty() {
        return vec![0; WAVEFORM_POINTS];
    }
    let n = levels.len();
    let buckets: Vec<f32> = (0..WAVEFORM_POINTS)
        .map(|i| {
            let from = i * n / WAVEFORM_POINTS;
            let to = ((i + 1) * n / WAVEFORM_POINTS).clamp(from + 1, n);
            let from = from.min(n - 1);
            levels[from..to].iter().sum::<f32>() / (to - from) as f32
        })
        .collect();
    let max = buckets.iter().cloned().fold(0.0f32, f32::max);
    if max < 1e-4 {
        return vec![0; WAVEFORM_POINTS];
    }
    buckets
        .iter()
        .map(|b| ((b / max) * 100.0).round().clamp(0.0, 100.0) as u8)
        .collect()
}

fn run(
    path: PathBuf,
    shared: Arc<Shared>,
    ready: Sender<Result<(), AudioError>>,
) -> Result<Outcome, AudioError> {
    let (rx, rate, stream) = match open_input(&shared) {
        Ok(v) => v,
        Err(e) => {
            let _ = ready.send(Err(e.clone()));
            return Err(e);
        }
    };
    let writer = File::create(&path)
        .map_err(|e| AudioError::Io(e.to_string()))
        .and_then(|f| OggOpusWriter::new(BufWriter::new(f)));
    let mut writer = match writer {
        Ok(w) => w,
        Err(e) => {
            let _ = ready.send(Err(e.clone()));
            return Err(e);
        }
    };
    if let Err(e) = stream.play().map_err(|e| map_cpal(e, true)) {
        let _ = ready.send(Err(e.clone()));
        return Err(e);
    }
    let _ = ready.send(Ok(()));

    let mut resampler = Resampler::new(rate, RATE);
    let mut converted = Vec::new();
    let mut pending: Vec<f32> = Vec::new();
    let mut levels = Vec::new();
    let mut total = 0u64;
    let mut failure = None;

    let mut feed = |converted: &mut Vec<f32>,
                    pending: &mut Vec<f32>,
                    flush: bool|
     -> Result<bool, AudioError> {
        pending.append(converted);
        let mut used = 0;
        while pending.len() - used >= FRAME || (flush && used < pending.len()) {
            let remaining = pending.len() - used;
            let take = remaining.min(FRAME);
            let allowed = (MAX_SAMPLES - total) as usize;
            let real = take.min(allowed);
            let mut frame = [0f32; FRAME];
            frame[..real].copy_from_slice(&pending[used..used + real]);
            used += take;
            if real == 0 {
                pending.clear();
                return Ok(false);
            }
            levels.push((frame[..real].iter().map(|s| s * s).sum::<f32>() / real as f32).sqrt());
            writer.encode_frame(&frame, real)?;
            total += real as u64;
            shared.samples.store(total, Ordering::Relaxed);
            if total >= MAX_SAMPLES {
                pending.clear();
                return Ok(false);
            }
        }
        pending.drain(..used);
        Ok(true)
    };

    let mut limit_reached = false;
    while !shared.stop.load(Ordering::Relaxed) {
        match rx.recv_timeout(Duration::from_millis(50)) {
            Ok(chunk) => {
                converted.clear();
                resampler.process(&chunk, &mut converted);
                match feed(&mut converted, &mut pending, false) {
                    Ok(true) => {}
                    Ok(false) => {
                        limit_reached = true;
                        break;
                    }
                    Err(e) => {
                        failure = Some(e);
                        break;
                    }
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
    drop(stream);
    if failure.is_none() && !limit_reached {
        for chunk in rx.try_iter() {
            converted.clear();
            resampler.process(&chunk, &mut converted);
            if let Err(e) = feed(&mut converted, &mut pending, false) {
                failure = Some(e);
                break;
            }
        }
        if failure.is_none() {
            converted.clear();
            resampler.flush(&mut converted);
            if let Err(e) = feed(&mut converted, &mut pending, true) {
                failure = Some(e);
            }
        }
    }
    if limit_reached && let Ok(mut slot) = shared.error.lock() {
        *slot = Some("Limite de 30 minutos de gravação atingido.".into());
    }
    if let Some(e) = failure {
        return Err(e);
    }
    let samples = total;
    writer.finish()?;
    Ok(Outcome { samples, levels })
}

fn open_input(shared: &Arc<Shared>) -> Result<(Receiver<Vec<f32>>, u32, cpal::Stream), AudioError> {
    let host = cpal::default_host();
    let device = host
        .default_input_device()
        .ok_or(AudioError::NoInputDevice)?;
    let supported = device
        .default_input_config()
        .map_err(|e| map_cpal(e, true))?;
    let format = supported.sample_format();
    let config: StreamConfig = supported.into();
    let rate = config.sample_rate;
    let channels = config.channels as usize;
    let (tx, rx) = mpsc::channel::<Vec<f32>>();
    let stream = match format {
        SampleFormat::F32 => build::<f32>(&device, &config, channels, tx, shared),
        SampleFormat::I16 => build::<i16>(&device, &config, channels, tx, shared),
        SampleFormat::I32 => build::<i32>(&device, &config, channels, tx, shared),
        SampleFormat::I8 => build::<i8>(&device, &config, channels, tx, shared),
        SampleFormat::U8 => build::<u8>(&device, &config, channels, tx, shared),
        SampleFormat::U16 => build::<u16>(&device, &config, channels, tx, shared),
        SampleFormat::F64 => build::<f64>(&device, &config, channels, tx, shared),
        other => Err(AudioError::Format(format!("amostra {other} não suportada"))),
    }?;
    Ok((rx, rate, stream))
}

fn build<T>(
    device: &Device,
    config: &StreamConfig,
    channels: usize,
    tx: Sender<Vec<f32>>,
    shared: &Arc<Shared>,
) -> Result<cpal::Stream, AudioError>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    let shared = shared.clone();
    device
        .build_input_stream(
            *config,
            move |data: &[T], _| {
                let mono: Vec<f32> = data
                    .chunks_exact(channels)
                    .map(|f| f.iter().map(|s| f32::from_sample(*s)).sum::<f32>() / channels as f32)
                    .collect();
                let _ = tx.send(mono);
            },
            move |e| {
                if let Ok(mut slot) = shared.error.lock() {
                    *slot = Some(format!("Falha no microfone: {e}"));
                }
            },
            None,
        )
        .map_err(|e| map_cpal(e, true))
}
