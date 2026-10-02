//! Default-microphone capture (cpal/WASAPI), delivered as mono 16-bit
//! chunks at the device's own sample rate — Vosk resamples internally.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::Arc;
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{ErrorKind, FromSample, Sample, SampleFormat, SizedSample};

/// Chunks buffered between the audio callback and recognition (~a few
/// seconds). If recognition ever falls that far behind, audio is dropped
/// rather than letting memory grow.
const QUEUE_CHUNKS: usize = 256;

/// Owns the open input stream; dropping it releases the microphone.
pub struct Microphone {
    _stream: cpal::Stream,
    chunks: Receiver<Vec<i16>>,
    failed: Arc<AtomicBool>,
    sample_rate: u32,
}

impl Microphone {
    pub fn open() -> Result<Self, String> {
        let device = cpal::default_host()
            .default_input_device()
            .ok_or_else(|| "No microphone found.".to_string())?;
        let supported = device
            .default_input_config()
            .map_err(|err| format!("Couldn't open the microphone: {err}"))?;

        let channels = usize::from(supported.channels()).max(1);
        let sample_rate = supported.sample_rate();
        let config = supported.config();
        let (sender, chunks) = mpsc::sync_channel(QUEUE_CHUNKS);
        let failed = Arc::new(AtomicBool::new(false));

        let stream = {
            let failed = failed.clone();
            let on_error = move |err: cpal::Error| {
                // Glitches (e.g. Xrun: a few dropped samples) are harmless for
                // speech; only a stream that can no longer deliver audio is
                // worth reopening.
                let fatal = matches!(
                    err.kind(),
                    ErrorKind::DeviceNotAvailable
                        | ErrorKind::DeviceChanged
                        | ErrorKind::StreamInvalidated
                        | ErrorKind::PermissionDenied
                        | ErrorKind::DeviceBusy
                );
                if fatal {
                    eprintln!("[voice] microphone error: {err}");
                    failed.store(true, Ordering::Relaxed);
                }
            };
            let on_chunk = move |chunk: Vec<i16>| {
                let _ = sender.try_send(chunk);
            };
            match supported.sample_format() {
                SampleFormat::F32 => build::<f32>(&device, config, channels, on_chunk, on_error),
                SampleFormat::I16 => build::<i16>(&device, config, channels, on_chunk, on_error),
                SampleFormat::U16 => build::<u16>(&device, config, channels, on_chunk, on_error),
                SampleFormat::I32 => build::<i32>(&device, config, channels, on_chunk, on_error),
                other => return Err(format!("Unsupported microphone format ({other:?}).")),
            }?
        };
        stream
            .play()
            .map_err(|err| format!("Couldn't start the microphone: {err}"))?;

        Ok(Self {
            _stream: stream,
            chunks,
            failed,
            sample_rate,
        })
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Next chunk of audio, or `None` if none arrived within `timeout`.
    pub fn next(&self, timeout: Duration) -> Result<Option<Vec<i16>>, String> {
        if self.failed.load(Ordering::Relaxed) {
            return Err("The microphone stopped working (unplugged?).".to_string());
        }
        match self.chunks.recv_timeout(timeout) {
            Ok(chunk) => Ok(Some(chunk)),
            Err(RecvTimeoutError::Timeout) => Ok(None),
            Err(RecvTimeoutError::Disconnected) => Err("The microphone stopped working.".to_string()),
        }
    }

    /// Discards audio captured while it wasn't being listened to.
    pub fn drain(&self) {
        while self.chunks.try_recv().is_ok() {}
    }
}

fn build<T>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    channels: usize,
    on_chunk: impl Fn(Vec<i16>) + Send + 'static,
    on_error: impl FnMut(cpal::Error) + Send + 'static,
) -> Result<cpal::Stream, String>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    device
        .build_input_stream::<T, _, _>(
            config,
            move |data: &[T], _| {
                // Down-mix every frame to mono.
                let chunk = data
                    .chunks(channels)
                    .map(|frame| {
                        let sum: f32 = frame.iter().map(|&s| f32::from_sample(s)).sum();
                        i16::from_sample(sum / frame.len() as f32)
                    })
                    .collect();
                on_chunk(chunk);
            },
            on_error,
            None,
        )
        .map_err(|err| format!("Couldn't open the microphone: {err}"))
}
