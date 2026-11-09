//! `nixon://` deep-link formatting. Links only navigate; the app resolves the time to a
//! transcript segment on open.

use percent_encoding::{utf8_percent_encode, AsciiSet, NON_ALPHANUMERIC};

/// Keep unreserved characters readable, encode everything else (notably `/ ? #`).
const ID_SET: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'~');

pub fn meeting_link(scheme: &str, meeting_id: &str) -> String {
    format!(
        "{scheme}://meeting/{}",
        utf8_percent_encode(meeting_id, ID_SET)
    )
}

/// Link to a moment in the meeting; `None` (no audio time) falls back to the meeting link.
/// Seconds are floored to tenths, so the link never lands after the segment starts.
pub fn moment_link(scheme: &str, meeting_id: &str, audio_start_time: Option<f64>) -> String {
    match audio_start_time {
        Some(t) if t.is_finite() && t >= 0.0 => {
            let tenths = (t * 10.0).floor() as i64;
            format!(
                "{}?t={}.{}",
                meeting_link(scheme, meeting_id),
                tenths / 10,
                tenths % 10
            )
        }
        _ => meeting_link(scheme, meeting_id),
    }
}

/// `mm:ss`, or `h:mm:ss` from one hour.
pub fn clock(seconds: f64) -> String {
    let total = if seconds.is_finite() && seconds > 0.0 {
        seconds as u64
    } else {
        0
    };
    let (h, m, s) = (total / 3600, (total / 60) % 60, total % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m:02}:{s:02}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn moment_link_floors_to_tenths() {
        assert_eq!(
            moment_link("nixon", "m1", Some(754.29)),
            "nixon://meeting/m1?t=754.2"
        );
        assert_eq!(
            moment_link("nixon-dev", "m1", Some(3.0)),
            "nixon-dev://meeting/m1?t=3.0"
        );
    }

    #[test]
    fn null_time_gives_meeting_link() {
        assert_eq!(moment_link("nixon", "m1", None), "nixon://meeting/m1");
    }

    #[test]
    fn encodes_id() {
        assert_eq!(
            meeting_link("nixon", "a b/c?d"),
            "nixon://meeting/a%20b%2Fc%3Fd"
        );
        assert_eq!(
            meeting_link("nixon", "meeting-1_x"),
            "nixon://meeting/meeting-1_x"
        );
    }

    #[test]
    fn clock_formats() {
        assert_eq!(clock(65.0), "01:05");
        assert_eq!(clock(3725.0), "1:02:05");
    }
}
