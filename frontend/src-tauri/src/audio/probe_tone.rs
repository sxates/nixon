// specs/0061 W3: onboarding Audio Capture probe — tone generator + playback.
//
// The pure waveform math (`sine_samples`) is deliberately kept separate from
// the device I/O (`play_440_for_ms`) so it can be unit-tested without ever
// touching real audio hardware. `audio::permissions::classify` is what turns
// what the tap actually hears back into a `PermissionProbe`.

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, StreamConfig};
use log::warn;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

/// Frequency of the onboarding probe tone.
const PROBE_FREQUENCY_HZ: f32 = 440.0;

/// Amplitude of the probe tone. Kept low on purpose: this plays through the
/// user's actual speakers during onboarding, so it must be audible to the
/// microphone/tap without being startling.
const PROBE_GAIN: f32 = 0.05;

/// Generate `ms` milliseconds of a `PROBE_FREQUENCY_HZ` sine wave at
/// `sample_rate`, scaled to `gain`. Pure function, no I/O — safe to call from
/// unit tests.
pub fn sine_samples(sample_rate: u32, ms: u32, gain: f32) -> Vec<f32> {
    let sample_count = (sample_rate as u64 * ms as u64 / 1000) as usize;
    (0..sample_count)
        .map(|i| {
            let t = i as f32 / sample_rate as f32;
            gain * (2.0 * std::f32::consts::PI * PROBE_FREQUENCY_HZ * t).sin()
        })
        .collect()
}

/// Play a brief, quiet 440 Hz tone through the default output device so the
/// onboarding Audio Capture probe has real audio to pick up on the tap.
/// Blocks the calling thread for approximately `ms` milliseconds; intended to
/// be run on its own thread (see `permissions::probe_audio_capture_sync`).
///
/// Any device/stream error is swallowed (logged as a warning) rather than
/// propagated: if we can't play a tone, the tap will simply read silence and
/// the probe will honestly classify that as `Silent`, not `Failed`.
pub fn play_440_for_ms(ms: u32) {
    let host = cpal::default_host();
    let device = match host.default_output_device() {
        Some(d) => d,
        None => {
            warn!("probe_tone: no default output device; playing nothing");
            return;
        }
    };

    let config = match device.default_output_config() {
        Ok(c) => c,
        Err(e) => {
            warn!("probe_tone: no default output config: {e}");
            return;
        }
    };

    let sample_rate = config.sample_rate().0;
    let channels = config.channels() as usize;
    let sample_format = config.sample_format();
    let stream_config: StreamConfig = config.into();

    let samples = sine_samples(sample_rate, ms, PROBE_GAIN);
    let position = std::sync::Arc::new(AtomicUsize::new(0));

    let err_fn = |err: cpal::StreamError| warn!("probe_tone: output stream error: {err}");

    let build_result = match sample_format {
        SampleFormat::F32 => {
            let samples = samples.clone();
            let position = position.clone();
            device.build_output_stream(
                &stream_config,
                move |data: &mut [f32], _| write_frames(data, &samples, &position, channels, 0.0),
                err_fn,
                None,
            )
        }
        SampleFormat::I16 => {
            let samples = samples.clone();
            let position = position.clone();
            device.build_output_stream(
                &stream_config,
                move |data: &mut [i16], _| write_frames(data, &samples, &position, channels, 0i16),
                err_fn,
                None,
            )
        }
        SampleFormat::U16 => {
            let samples = samples.clone();
            let position = position.clone();
            device.build_output_stream(
                &stream_config,
                move |data: &mut [u16], _| {
                    write_frames(data, &samples, &position, channels, u16::MAX / 2)
                },
                err_fn,
                None,
            )
        }
        other => {
            warn!("probe_tone: unsupported output sample format {other:?}");
            return;
        }
    };

    let stream = match build_result {
        Ok(s) => s,
        Err(e) => {
            warn!("probe_tone: failed to build output stream: {e}");
            return;
        }
    };

    if let Err(e) = stream.play() {
        warn!("probe_tone: failed to start output stream: {e}");
        return;
    }

    std::thread::sleep(Duration::from_millis(ms as u64));
    // `stream` drops here, tearing down playback.
}

/// Write one probe sample per output frame, converting from the `f32`
/// waveform into whatever sample type the device's default config wants.
/// Falls back to `silence` once the tone buffer is exhausted.
fn write_frames<T>(
    data: &mut [T],
    samples: &[f32],
    position: &AtomicUsize,
    channels: usize,
    silence: T,
) where
    T: cpal::Sample + cpal::FromSample<f32>,
{
    for frame in data.chunks_mut(channels.max(1)) {
        let idx = position.fetch_add(1, Ordering::Relaxed);
        let value = samples
            .get(idx)
            .map(|s| T::from_sample(*s))
            .unwrap_or(silence);
        for out in frame.iter_mut() {
            *out = value;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sine_samples_has_expected_length() {
        assert_eq!(sine_samples(48_000, 300, 0.05).len(), 14_400);
    }

    #[test]
    fn sine_samples_never_exceeds_gain() {
        for sample in sine_samples(48_000, 300, 0.05) {
            assert!(sample.abs() <= 0.05, "sample {sample} exceeds gain 0.05");
        }
    }
}
