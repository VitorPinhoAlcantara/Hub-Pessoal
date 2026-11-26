use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Device, FromSample, SampleFormat, SizedSample, StreamConfig};

use super::opus_file::decode_file;
use super::resample::RATE;
use super::{AudioError, map_cpal};

const NO_SEEK: u64 = u64::MAX;

pub(super) struct Shared {
    pub(super) pos: AtomicU64,
    pub(super) seek: AtomicU64,
    pub(super) paused: AtomicBool,
    pub(super) finished: AtomicBool,
    pub(super) speed: AtomicU32,
    len: usize,
}

pub struct Playback {
    shared: Arc<Shared>,
    stop: Option<mpsc::Sender<()>>,
    handle: Option<JoinHandle<()>>,
}

impl Shared {
    pub(super) fn new(len: usize) -> Self {
        Self {
            pos: AtomicU64::new(0f64.to_bits()),
            seek: AtomicU64::new(NO_SEEK),
            paused: AtomicBool::new(false),
            finished: AtomicBool::new(false),
            speed: AtomicU32::new(1.0f32.to_bits()),
            len,
        }
    }
}

impl Playback {
    pub fn start(path: &Path) -> Result<Playback, AudioError> {
        let samples: Arc<[i16]> = decode_file(path)?.into();
        if samples.is_empty() {
            return Err(AudioError::Empty);
        }
        let shared = Arc::new(Shared::new(samples.len()));
        let (ready_tx, ready_rx) = mpsc::channel();
        let (stop_tx, stop_rx) = mpsc::channel::<()>();
        let thread_shared = shared.clone();
        let handle = std::thread::Builder::new()
            .name("hub-audio-play".into())
            .spawn(move || match open_output(samples, thread_shared) {
                Ok(stream) => {
                    let _ = ready_tx.send(Ok(()));
                    let _ = stop_rx.recv();
                    drop(stream);
                }
                Err(e) => {
                    let _ = ready_tx.send(Err(e));
                }
            })
            .map_err(|e| AudioError::Device(e.to_string()))?;
        match ready_rx.recv() {
            Ok(Ok(())) => Ok(Playback {
                shared,
                stop: Some(stop_tx),
                handle: Some(handle),
            }),
            Ok(Err(e)) => {
                let _ = handle.join();
                Err(e)
            }
            Err(_) => {
                let _ = handle.join();
                Err(AudioError::Device("a reprodução não pôde iniciar".into()))
            }
        }
    }

    pub fn pause(&self) {
        self.shared.paused.store(true, Ordering::Relaxed);
    }

    pub fn resume(&self) {
        self.shared.paused.store(false, Ordering::Relaxed);
    }

    pub fn is_paused(&self) -> bool {
        self.shared.paused.load(Ordering::Relaxed)
    }

    pub fn seek(&self, to: Duration) {
        let target = (to.as_secs_f64() * RATE as f64).min(self.shared.len as f64);
        self.shared.finished.store(false, Ordering::Relaxed);
        self.shared.pos.store(target.to_bits(), Ordering::Relaxed);
        self.shared.seek.store(target.to_bits(), Ordering::Relaxed);
    }

    pub fn set_speed(&self, speed: f32) {
        let speed = if speed.is_finite() {
            speed.clamp(0.25, 4.0)
        } else {
            1.0
        };
        self.shared.speed.store(speed.to_bits(), Ordering::Relaxed);
    }

    pub fn position(&self) -> Duration {
        let pos =
            f64::from_bits(self.shared.pos.load(Ordering::Relaxed)).min(self.shared.len as f64);
        Duration::from_secs_f64(pos / RATE as f64)
    }

    pub fn duration(&self) -> Duration {
        Duration::from_secs_f64(self.shared.len as f64 / RATE as f64)
    }

    pub fn finished(&self) -> bool {
        self.shared.finished.load(Ordering::Relaxed)
    }
}

impl Drop for Playback {
    fn drop(&mut self) {
        drop(self.stop.take());
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

fn open_output(samples: Arc<[i16]>, shared: Arc<Shared>) -> Result<cpal::Stream, AudioError> {
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or(AudioError::NoOutputDevice)?;
    let supported = device
        .default_output_config()
        .map_err(|e| map_cpal(e, false))?;
    let format = supported.sample_format();
    let config: StreamConfig = supported.into();
    let stream = match format {
        SampleFormat::F32 => build::<f32>(&device, &config, samples, shared),
        SampleFormat::I16 => build::<i16>(&device, &config, samples, shared),
        SampleFormat::I32 => build::<i32>(&device, &config, samples, shared),
        SampleFormat::I8 => build::<i8>(&device, &config, samples, shared),
        SampleFormat::U8 => build::<u8>(&device, &config, samples, shared),
        SampleFormat::U16 => build::<u16>(&device, &config, samples, shared),
        SampleFormat::F64 => build::<f64>(&device, &config, samples, shared),
        other => Err(AudioError::Format(format!("amostra {other} não suportada"))),
    }?;
    stream.play().map_err(|e| map_cpal(e, false))?;
    Ok(stream)
}

fn build<T>(
    device: &Device,
    config: &StreamConfig,
    samples: Arc<[i16]>,
    shared: Arc<Shared>,
) -> Result<cpal::Stream, AudioError>
where
    T: SizedSample + FromSample<f32>,
{
    let channels = config.channels as usize;
    let rate = config.sample_rate as f64;
    device
        .build_output_stream(
            *config,
            move |out: &mut [T], _| fill(out, channels, rate, &samples, &shared),
            |_| {},
            None,
        )
        .map_err(|e| map_cpal(e, false))
}

pub(super) fn fill<T: SizedSample + FromSample<f32>>(
    out: &mut [T],
    channels: usize,
    rate: f64,
    samples: &[i16],
    shared: &Shared,
) {
    let requested = shared.seek.swap(NO_SEEK, Ordering::Relaxed);
    let mut pos = if requested != NO_SEEK {
        f64::from_bits(requested)
    } else {
        f64::from_bits(shared.pos.load(Ordering::Relaxed))
    };
    let paused = shared.paused.load(Ordering::Relaxed);
    let step = f32::from_bits(shared.speed.load(Ordering::Relaxed)) as f64 * RATE as f64 / rate;
    let len = samples.len();
    for frame in out.chunks_mut(channels.max(1)) {
        let mut value = 0.0f32;
        if !paused && (pos as usize) < len {
            let i = pos as usize;
            let a = samples[i] as f32;
            let b = samples.get(i + 1).map_or(a, |s| *s as f32);
            value = (a + (b - a) * (pos - i as f64) as f32) / 32768.0;
            pos += step;
            if pos as usize >= len {
                shared.finished.store(true, Ordering::Relaxed);
            }
        }
        let sample = T::from_sample(value);
        for slot in frame.iter_mut() {
            *slot = sample;
        }
    }
    shared.pos.store(pos.to_bits(), Ordering::Relaxed);
}
