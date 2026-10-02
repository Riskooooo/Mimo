//! Short UI sounds (wake chime, success pop, error), synthesized here and
//! played natively on the default output device with cpal. Web Audio inside
//! the WebView turned out to be silent on real setups (rendering fine,
//! nothing audible), so audio stays on the Rust side.
//!
//! A dedicated thread opens an output stream per sound and drops it once
//! the sound is over, so nothing keeps the audio device busy in between.

use std::f32::consts::TAU;
use std::sync::mpsc::{self, Sender};
use std::sync::Arc;
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample};
use serde::Deserialize;

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Sound {
    /// "I'm listening": two rising notes a fifth apart.
    Wake,
    /// "Done": a quick upward pop.
    Success,
    /// "Didn't work": two soft falling notes.
    Error,
    /// A reminder is due: a gentle three-note bell.
    Reminder,
    /// Alarm ring, repeated by the pill until dismissed: three bright beeps.
    Alarm,
}

#[derive(Clone, Copy)]
enum Wave {
    Sine,
    Triangle,
}

struct Note {
    /// Start frequency (Hz), gliding to `to` over the first 60% of the note —
    /// the glide is what makes it feel "juicy" rather than flat.
    from: f32,
    to: f32,
    /// Offset from the start of the sound, and length (s).
    at: f32,
    length: f32,
    gain: f32,
    wave: Wave,
}

const fn note(from: f32, to: f32, at: f32, length: f32, gain: f32, wave: Wave) -> Note {
    Note { from, to, at, length, gain, wave }
}

const WAKE: &[Note] = &[
    note(740.0, 784.0, 0.0, 0.2, 0.22, Wave::Triangle),
    note(1108.0, 1175.0, 0.11, 0.32, 0.2, Wave::Triangle),
];
const SUCCESS: &[Note] = &[note(620.0, 1240.0, 0.0, 0.2, 0.22, Wave::Sine)];
const ERROR: &[Note] = &[
    note(440.0, 415.0, 0.0, 0.2, 0.2, Wave::Triangle),
    note(330.0, 311.0, 0.15, 0.28, 0.2, Wave::Triangle),
];

const REMINDER: &[Note] = &[
    note(784.0, 784.0, 0.0, 0.35, 0.2, Wave::Sine),
    note(988.0, 988.0, 0.16, 0.35, 0.2, Wave::Sine),
    note(1175.0, 1175.0, 0.32, 0.5, 0.2, Wave::Sine),
];
const ALARM: &[Note] = &[
    note(1046.0, 1046.0, 0.0, 0.14, 0.24, Wave::Triangle),
    note(1046.0, 1046.0, 0.2, 0.14, 0.24, Wave::Triangle),
    note(1318.0, 1318.0, 0.4, 0.22, 0.24, Wave::Triangle),
];

fn notes(sound: Sound) -> &'static [Note] {
    match sound {
        Sound::Wake => WAKE,
        Sound::Success => SUCCESS,
        Sound::Error => ERROR,
        Sound::Reminder => REMINDER,
        Sound::Alarm => ALARM,
    }
}

/// Silence at the start of each stream: some wireless headsets swallow the
/// first moments of a freshly opened stream while their link wakes up.
const LEAD_IN_S: f32 = 0.15;

/// Soft-edge echo tail appended after the last note.
const ECHO_DELAY_S: f32 = 0.09;
const ECHO_MIX: f32 = 0.18;
/// One-pole low-pass cutoff, to round off the raw oscillators.
const LOWPASS_HZ: f32 = 4200.0;

/// Renders a sound to mono samples at `rate`.
fn render(sound: Sound, rate: f32) -> Vec<f32> {
    let notes = notes(sound);
    let end = notes.iter().map(|n| n.at + n.length).fold(0.0, f32::max);
    let lead_in = (LEAD_IN_S * rate) as usize;
    let total = lead_in + ((end + ECHO_DELAY_S + 0.05) * rate) as usize;
    let mut dry = vec![0.0f32; total];

    for n in notes {
        let start = lead_in + (n.at * rate) as usize;
        let len = (n.length * rate) as usize;
        let glide = (n.length * 0.6).max(f32::EPSILON);
        let mut phase = 0.0f32;
        for i in 0..len.min(total.saturating_sub(start)) {
            let t = i as f32 / rate;
            // Exponential glide, then hold.
            let freq = n.from * (n.to / n.from).powf((t / glide).min(1.0));
            phase = (phase + freq / rate).fract();
            let osc = match n.wave {
                Wave::Sine => (phase * TAU).sin(),
                Wave::Triangle => 1.0 - 4.0 * (phase - 0.5).abs(),
            };
            // 12 ms attack, then a curved fade that keeps the note present
            // for most of its length (an exponential fade to silence sounded
            // like a click: half-way through it was already ~-33 dB).
            let attack = 0.012;
            let env = if t < attack {
                n.gain * (t / attack)
            } else {
                let remaining = 1.0 - (t - attack) / (n.length - attack);
                n.gain * remaining.max(0.0).powf(1.6)
            };
            dry[start + i] += osc * env;
        }
    }

    let alpha = 1.0 - (-TAU * LOWPASS_HZ / rate).exp();
    let mut low = 0.0f32;
    for s in &mut dry {
        low += alpha * (*s - low);
        *s = low;
    }

    let delay = (ECHO_DELAY_S * rate) as usize;
    let mut out = dry.clone();
    for i in delay..total {
        out[i] += dry[i - delay] * ECHO_MIX;
    }
    out
}

/// Handle to the sound thread, kept in Tauri's managed state.
pub struct Sounds {
    queue: Sender<Sound>,
}

impl Sounds {
    pub fn new() -> Self {
        let (queue, requests) = mpsc::channel::<Sound>();
        let _ = std::thread::Builder::new()
            .name("mimo-sound".into())
            .spawn(move || {
                for sound in requests {
                    if let Err(err) = play_blocking(sound) {
                        eprintln!("[sound] {err}");
                    }
                }
            });
        Self { queue }
    }

    pub fn play(&self, sound: Sound) {
        let _ = self.queue.send(sound);
    }
}

fn play_blocking(sound: Sound) -> Result<(), String> {
    let device = cpal::default_host()
        .default_output_device()
        .ok_or_else(|| "no audio output device".to_string())?;
    let supported = device
        .default_output_config()
        .map_err(|err| format!("output config: {err}"))?;
    let channels = usize::from(supported.channels()).max(1);
    let rate = supported.sample_rate() as f32;
    let samples = Arc::new(render(sound, rate));
    let duration = Duration::from_secs_f32(samples.len() as f32 / rate);
    let config = supported.config();

    let stream = match supported.sample_format() {
        SampleFormat::F32 => build::<f32>(&device, config, channels, samples),
        SampleFormat::I16 => build::<i16>(&device, config, channels, samples),
        SampleFormat::U16 => build::<u16>(&device, config, channels, samples),
        SampleFormat::I32 => build::<i32>(&device, config, channels, samples),
        other => return Err(format!("unsupported output format {other:?}")),
    }?;
    stream.play().map_err(|err| format!("play: {err}"))?;
    // Let it finish (plus device latency) before closing the stream.
    std::thread::sleep(duration + Duration::from_millis(150));
    Ok(())
}

fn build<T>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    channels: usize,
    samples: Arc<Vec<f32>>,
) -> Result<cpal::Stream, String>
where
    T: SizedSample + FromSample<f32>,
{
    let mut position = 0usize;
    device
        .build_output_stream::<T, _, _>(
            config,
            move |out: &mut [T], _| {
                for frame in out.chunks_mut(channels) {
                    let value = samples.get(position).copied().unwrap_or(0.0);
                    position += 1;
                    for slot in frame {
                        *slot = T::from_sample(value);
                    }
                }
            },
            |err| eprintln!("[sound] stream error: {err}"),
            None,
        )
        .map_err(|err| format!("open output: {err}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sounds_render_audible_and_unclipped() {
        let rate = 48_000.0;
        for sound in [Sound::Wake, Sound::Success, Sound::Error, Sound::Reminder, Sound::Alarm] {
            let samples = render(sound, rate);
            let peak = samples.iter().fold(0.0f32, |m, s| m.max(s.abs()));
            assert!(peak < 1.0, "{sound:?} clips: {peak}");

            // Still clearly audible well past the attack (the original fade
            // made everything after ~30 ms near-silent, and inaudible on a
            // real headset).
            let window = &samples[((LEAD_IN_S + 0.05) * rate) as usize..((LEAD_IN_S + 0.15) * rate) as usize];
            let rms = (window.iter().map(|s| s * s).sum::<f32>() / window.len() as f32).sqrt();
            assert!(rms > 0.03, "{sound:?} fades too fast: rms {rms}");
        }
    }
}
