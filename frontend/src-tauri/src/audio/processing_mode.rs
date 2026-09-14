//! Effective processing-mode decision (low-power-mode spec §§2–3).
//!
//! Pure so it is unit-testable: the caller supplies the global preferences,
//! the live power state, and the meeting's `processing_mode` override.

/// `meetings.processing_mode` value forcing full live processing.
pub const MODE_LIVE: &str = "live";
/// `meetings.processing_mode` value forcing deferral (record-only).
pub const MODE_DEFER: &str = "defer";

/// Should this recording session run live VAD/STT?
///
/// Precedence: per-meeting override → low-power-on-battery → the global
/// `live_transcription_enabled` preference (specs/0029 WS7.2 record-only mode).
pub fn effective_live_stt(
    live_transcription_enabled: bool,
    low_power_on_battery: bool,
    on_battery: bool,
    meeting_override: Option<&str>,
) -> bool {
    match meeting_override {
        Some(MODE_LIVE) => true,
        Some(MODE_DEFER) => false,
        _ => live_transcription_enabled && !(low_power_on_battery && on_battery),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn override_wins_in_both_directions() {
        // 'live' forces live even on battery with low-power on…
        assert!(effective_live_stt(true, true, true, Some("live")));
        // …and even when the global record-only preference is set.
        assert!(effective_live_stt(false, true, false, Some("live")));
        // 'defer' forces deferral even on AC.
        assert!(!effective_live_stt(true, true, false, Some("defer")));
    }

    #[test]
    fn low_power_defers_only_on_battery() {
        assert!(
            !effective_live_stt(true, true, true, None),
            "battery + switch on → defer"
        );
        assert!(effective_live_stt(true, true, false, None), "AC → live");
        assert!(
            effective_live_stt(true, false, true, None),
            "switch off → live on battery"
        );
    }

    #[test]
    fn record_only_preference_still_defers() {
        assert!(!effective_live_stt(false, false, false, None));
    }

    #[test]
    fn unknown_override_falls_back_to_global() {
        assert!(effective_live_stt(true, true, false, Some("bogus")));
    }
}
