//! specs/0057 §3.2 (Plan 3 residual): the live meter feed behind the record page's two
//! needle VUs (CH1 MIC / CH2 SYS) and the transport rail's ladder.
//!
//! The pipeline stages one 600 ms window per mix — the CLEAN pre-mix mic and system
//! windows plus the mixed result — and this stage walks all three in ~80 ms sub-frames
//! paced by `LIVE_METER_EMIT_INTERVAL`, so the meters animate at real-time cadence
//! (delayed by one mix window) instead of pulsing once per window. Lives here rather
//! than in `pipeline.rs` because the pipeline is at its size-ratchet ceiling.
//!
//! Event contract (`recording-level`, additive over the 0019/0029 shape):
//! `{ rms, peak, mic: { rms, peak }, sys: { rms, peak } }` — every value in 0..1.
//! `rms`/`peak` stay the MIXED levels so the rail ladder and any older consumer keep
//! working unchanged; the per-channel objects feed the two VUs.

use super::audio_processing::spectrum_bands;

/// Cadence of the live `recording-level` / `recording-spectrum` events (~12.5/s — the
/// same throttle these events had on the worker side before specs/0029 WS4.2, and
/// comfortably above the frontend hook's decay window).
pub const LIVE_METER_EMIT_INTERVAL: std::time::Duration = std::time::Duration::from_millis(80);

/// RMS + absolute peak of one frame, both clamped to 0..1. An empty frame is silence.
pub fn frame_level(frame: &[f32]) -> (f32, f32) {
    if frame.is_empty() {
        return (0.0, 0.0);
    }
    let rms = (frame.iter().map(|&s| s * s).sum::<f32>() / frame.len() as f32).sqrt();
    let peak = frame.iter().fold(0.0f32, |m, &s| m.max(s.abs()));
    (rms.clamp(0.0, 1.0), peak.clamp(0.0, 1.0))
}

/// The staged windows + walk position. One per pipeline; `stage` replaces (rather than
/// appends to) any unconsumed remainder so the meters always show the newest audio.
pub struct LiveMeterStage {
    sample_rate: u32,
    mic: Vec<f32>,
    sys: Vec<f32>,
    mixed: Vec<f32>,
    pos: usize,
    last_emit: std::time::Instant,
}

impl LiveMeterStage {
    pub fn new(sample_rate: u32) -> Self {
        Self {
            sample_rate,
            mic: Vec::new(),
            sys: Vec::new(),
            mixed: Vec::new(),
            pos: 0,
            // Starts one interval in the past so the very first staged window emits at once.
            last_emit: std::time::Instant::now()
                .checked_sub(LIVE_METER_EMIT_INTERVAL)
                .unwrap_or_else(std::time::Instant::now),
        }
    }

    /// Stage the freshest RAW windows. `mic`/`sys` are the clean pre-mix channels (already
    /// zero-padded to the window by the ring buffer); `mixed` is what the recording hears.
    pub fn stage(&mut self, mic: &[f32], sys: &[f32], mixed: &[f32]) {
        self.mic.clear();
        self.mic.extend_from_slice(mic);
        self.sys.clear();
        self.sys.extend_from_slice(sys);
        self.mixed.clear();
        self.mixed.extend_from_slice(mixed);
        self.pos = 0;
    }

    /// Emit one sub-frame's `recording-level` + `recording-spectrum` through `emit` if a
    /// sub-frame is staged and the throttle allows. Call on every pipeline loop iteration
    /// (including recv timeouts, so the staged window keeps draining during capture gaps).
    pub fn maybe_emit(&mut self, emit: &dyn Fn(&str, serde_json::Value)) {
        if self.pos >= self.mixed.len() {
            return; // staged window fully consumed — nothing new to show
        }
        if self.last_emit.elapsed() < LIVE_METER_EMIT_INTERVAL {
            return; // throttled
        }
        let frame_len =
            ((self.sample_rate as u128 * LIVE_METER_EMIT_INTERVAL.as_millis()) / 1000) as usize;
        let start = self.pos;
        let end = (start + frame_len.max(1)).min(self.mixed.len());
        self.pos = end;

        let sub =
            |v: &[f32]| -> (f32, f32) { frame_level(&v[start.min(v.len())..end.min(v.len())]) };
        let (rms, peak) = sub(&self.mixed);
        let (mic_rms, mic_peak) = sub(&self.mic);
        let (sys_rms, sys_peak) = sub(&self.sys);
        emit(
            "recording-level",
            serde_json::json!({
                "rms": rms,
                "peak": peak,
                "mic": { "rms": mic_rms, "peak": mic_peak },
                "sys": { "rms": sys_rms, "peak": sys_peak },
            }),
        );
        let bands = spectrum_bands(&self.mixed[start..end], self.sample_rate);
        emit("recording-spectrum", serde_json::json!({ "bands": bands }));

        self.last_emit = std::time::Instant::now();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    fn collect(stage: &mut LiveMeterStage) -> Vec<(String, serde_json::Value)> {
        let out = RefCell::new(Vec::new());
        stage.maybe_emit(&|name: &str, payload: serde_json::Value| {
            out.borrow_mut().push((name.to_string(), payload));
        });
        out.into_inner()
    }

    fn level_of(events: &[(String, serde_json::Value)]) -> &serde_json::Value {
        &events
            .iter()
            .find(|(n, _)| n == "recording-level")
            .expect("recording-level emitted")
            .1
    }

    #[test]
    fn frame_level_is_rms_and_abs_peak_clamped() {
        assert_eq!(frame_level(&[]), (0.0, 0.0));
        let (rms, peak) = frame_level(&[0.5, -0.5, 0.5, -0.5]);
        assert!((rms - 0.5).abs() < 1e-6);
        assert_eq!(peak, 0.5);
        // Over-full samples (the mixer soft-scales, but the raw channels may not) clamp.
        let (rms, peak) = frame_level(&[2.0, -2.0]);
        assert_eq!((rms, peak), (1.0, 1.0));
    }

    #[test]
    fn per_channel_levels_are_independent_of_the_mix() {
        let sr = 1000; // 80 ms sub-frame = 80 samples
        let mut stage = LiveMeterStage::new(sr);
        let mic = vec![0.5f32; 160];
        let sys = vec![0.0f32; 160];
        let mixed: Vec<f32> = mic.iter().zip(&sys).map(|(m, s)| m + s).collect();
        stage.stage(&mic, &sys, &mixed);

        let events = collect(&mut stage);
        let level = level_of(&events);
        let f = |p: &str| level.pointer(p).and_then(|v| v.as_f64()).unwrap();
        assert!((f("/mic/rms") - 0.5).abs() < 1e-6, "mic carries the signal");
        assert_eq!(f("/sys/rms"), 0.0, "system channel is silent");
        assert_eq!(f("/sys/peak"), 0.0);
        assert!(
            (f("/rms") - 0.5).abs() < 1e-6,
            "mixed level unchanged for older consumers"
        );
        assert!(events.iter().any(|(n, _)| n == "recording-spectrum"));
    }

    #[test]
    fn walks_the_staged_window_in_sub_frames_then_goes_quiet() {
        let sr = 1000;
        let mut stage = LiveMeterStage::new(sr);
        let win = vec![0.25f32; 200]; // 2.5 sub-frames → 3 emits
        stage.stage(&win, &win, &win);
        let mut emits = 0;
        for _ in 0..10 {
            stage.last_emit = std::time::Instant::now() - LIVE_METER_EMIT_INTERVAL; // defeat the throttle
            if !collect(&mut stage).is_empty() {
                emits += 1;
            }
        }
        assert_eq!(emits, 3);
    }

    #[test]
    fn throttles_between_sub_frames() {
        let mut stage = LiveMeterStage::new(1000);
        let win = vec![0.25f32; 400];
        stage.stage(&win, &win, &win);
        assert!(!collect(&mut stage).is_empty(), "first emit is immediate");
        assert!(
            collect(&mut stage).is_empty(),
            "second emit waits for the interval"
        );
    }

    #[test]
    fn short_channel_window_never_panics() {
        // Defensive: the ring buffer pads both channels, but a mismatched length must
        // degrade to silence for the short channel rather than index out of range.
        let mut stage = LiveMeterStage::new(1000);
        let mixed = vec![0.25f32; 160];
        stage.stage(&mixed[..10], &[], &mixed);
        let events = collect(&mut stage);
        let level = level_of(&events);
        assert_eq!(
            level.pointer("/sys/rms").and_then(|v| v.as_f64()),
            Some(0.0)
        );
        assert!(level.pointer("/mic/rms").and_then(|v| v.as_f64()).unwrap() > 0.0);
    }
}
