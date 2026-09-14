//! Pure folder-matching logic for recovering a meeting's recording folder when
//! `meetings.folder_path` is NULL (specs/0010 follow-up; diarization salvage).
//!
//! Recording folders are named by [`audio::audio_processing::create_meeting_folder`]
//! as `<meeting_name>_<UTC %Y-%m-%d_%H-%M>`, where the default `meeting_name`
//! (`audio::recording_commands`) is `Meeting <LOCAL %Y-%m-%d_%H-%M-%S>`. So a
//! default-named folder looks like:
//!
//! ```text
//! Meeting 2026-06-25_10-00-47_2026-06-25_17-00
//!         └──────── local start (s) ──┘ └ UTC start (min) ┘
//! ```
//!
//! The DB stores `created_at` in UTC. The strongest, most precise match signal is
//! the **first** timestamp token in the folder name: the LOCAL recording-start time
//! with seconds. We convert the meeting's `created_at` to local time and look for the
//! candidate whose embedded local timestamp is within a small tolerance. A meeting
//! `title` corroborates (and disambiguates) titled folders.
//!
//! This module is intentionally pure (no fs / DB): the filesystem scan and the
//! opportunistic backfill live in `pipeline.rs` and call [`best_folder_match`].

use chrono::{DateTime, Local, NaiveDateTime, Utc};

use crate::audio::audio_processing::sanitize_filename;

/// How far the folder's embedded local timestamp may differ from `created_at`
/// (converted to local) and still be considered the same recording. Covers
/// sub-second rounding plus the small gap between name generation and the DB write.
const MATCH_TOLERANCE_SECS: i64 = 90;

/// Parse the first `%Y-%m-%d_%H-%M-%S` token embedded in a folder name into a naive
/// (local, wall-clock) datetime. The folder name carries two timestamps; the first
/// is the local recording-start with seconds, the second is the UTC start at minute
/// precision (no seconds), so a strict seconds-precision parse only ever matches the
/// first one.
///
/// The folder name is split on `_` into parts; we scan for an adjacent
/// `YYYY-MM-DD` + `HH-MM-SS` pair (both validated by exact digit counts — chrono's
/// own parser is too lenient about field widths / leading whitespace to slide over a
/// raw substring). The minute-precision UTC suffix (`HH-MM`, two parts) never matches
/// the seconds pattern, so the local start is always found first.
fn parse_embedded_local_timestamp(folder_name: &str) -> Option<NaiveDateTime> {
    let parts: Vec<&str> = folder_name.split('_').collect();
    for pair in parts.windows(2) {
        // The date part may carry a non-date prefix (the meeting name, e.g.
        // "Meeting 2026-06-25"); take its trailing date-shaped token.
        let (Some(date), time) = (trailing_date_token(pair[0]), pair[1]) else {
            continue;
        };
        if is_time_token(time) {
            let combined = format!("{date}_{time}");
            if let Ok(dt) = NaiveDateTime::parse_from_str(&combined, "%Y-%m-%d_%H-%M-%S") {
                return Some(dt);
            }
        }
    }
    None
}

/// Return the trailing `YYYY-MM-DD` token of `s` (last 10 chars) iff it is a valid
/// date shape; the preceding chars (a meeting-name prefix) are ignored.
fn trailing_date_token(s: &str) -> Option<&str> {
    let date = s.get(s.len().checked_sub(10)?..)?;
    if is_date_token(date) {
        Some(date)
    } else {
        None
    }
}

/// `YYYY-MM-DD`: digits 4-2-2 split by `-`.
fn is_date_token(s: &str) -> bool {
    let f: Vec<&str> = s.split('-').collect();
    f.len() == 3
        && f[0].len() == 4
        && f[1].len() == 2
        && f[2].len() == 2
        && f.iter().all(|p| p.bytes().all(|b| b.is_ascii_digit()))
}

/// `HH-MM-SS`: digits 2-2-2 split by `-`.
fn is_time_token(s: &str) -> bool {
    let f: Vec<&str> = s.split('-').collect();
    f.len() == 3
        && f.iter()
            .all(|p| p.len() == 2 && p.bytes().all(|b| b.is_ascii_digit()))
}

/// Convert a UTC instant to the local naive wall-clock time, matching how the
/// recording path stamps folder names (`chrono::Local::now()`).
fn utc_to_local_naive(created_at: DateTime<Utc>) -> NaiveDateTime {
    created_at.with_timezone(&Local).naive_local()
}

/// Decide whether `folder_name` corroborates `title` (best-effort). The folder's
/// `meeting_name` prefix is the (sanitized) title; we accept the folder if its name
/// contains the sanitized title. Empty/auto titles ("Meeting ...") don't constrain.
fn title_corroborates(folder_name: &str, title: Option<&str>) -> bool {
    match title {
        Some(t) => {
            let sanitized = sanitize_filename(t);
            sanitized.is_empty() || folder_name.contains(&sanitized)
        }
        None => true,
    }
}

/// Pick the best-matching folder for a meeting from a list of candidate folder names.
///
/// Returns the chosen folder name, or `None` when the match is absent or ambiguous:
/// - parse each candidate's embedded local timestamp; keep those within
///   [`MATCH_TOLERANCE_SECS`] of `created_at` (converted to local);
/// - prefer candidates whose name also corroborates `title`;
/// - among the surviving set, pick the single closest by timestamp delta;
/// - if the two best candidates are equally close (a genuine tie), return `None`
///   rather than guess.
///
/// `created_at` is the meeting's UTC creation time; `title` is its (optional) title.
/// `candidates` are bare directory names under the recordings root.
pub fn best_folder_match(
    created_at: DateTime<Utc>,
    title: Option<&str>,
    candidates: &[String],
) -> Option<String> {
    let target_local = utc_to_local_naive(created_at);

    // (delta_secs, corroborates, name) for every candidate within tolerance.
    let mut scored: Vec<(i64, bool, &String)> = candidates
        .iter()
        .filter_map(|name| {
            let ts = parse_embedded_local_timestamp(name)?;
            let delta = (ts - target_local).num_seconds().abs();
            if delta <= MATCH_TOLERANCE_SECS {
                Some((delta, title_corroborates(name, title), name))
            } else {
                None
            }
        })
        .collect();

    if scored.is_empty() {
        return None;
    }

    // If a title was given and any candidate corroborates it, restrict to those —
    // a titled meeting should never resolve to an unrelated default-named folder.
    let has_title = title
        .map(|t| !sanitize_filename(t).is_empty())
        .unwrap_or(false);
    if has_title && scored.iter().any(|(_, corr, _)| *corr) {
        scored.retain(|(_, corr, _)| *corr);
    }

    // Sort by timestamp closeness (then corroboration as a stable secondary key).
    scored.sort_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)));

    // Ambiguity guard: if the two closest are equally close, refuse to guess unless
    // exactly one of them corroborates the title.
    if scored.len() >= 2 && scored[0].0 == scored[1].0 && scored[0].1 == scored[1].1 {
        return None;
    }

    Some(scored[0].2.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    /// Build a UTC datetime from components.
    fn utc(y: i32, mo: u32, d: u32, h: u32, mi: u32, s: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, mo, d, h, mi, s).unwrap()
    }

    /// The exact local-start string for a given UTC instant, as the recorder would
    /// stamp it. Keeps tests timezone-agnostic (they run wherever CI lives).
    fn local_start(created_at: DateTime<Utc>) -> String {
        utc_to_local_naive(created_at)
            .format("%Y-%m-%d_%H-%M-%S")
            .to_string()
    }

    /// The UTC minute suffix the recorder appends (`create_meeting_folder`).
    fn utc_minute(created_at: DateTime<Utc>) -> String {
        created_at.format("%Y-%m-%d_%H-%M").to_string()
    }

    /// Reconstruct a realistic default folder name for a meeting created at `ca`.
    fn default_folder(ca: DateTime<Utc>) -> String {
        format!("Meeting {}_{}", local_start(ca), utc_minute(ca))
    }

    #[test]
    fn matches_default_named_folder_by_local_timestamp() {
        // The user's real NULL meeting: created_at 2026-06-25T17:00:47Z.
        let ca = utc(2026, 6, 25, 17, 0, 47);
        let folder = default_folder(ca);
        let candidates = vec![
            default_folder(utc(2026, 6, 25, 16, 24, 45)),
            folder.clone(),
            default_folder(utc(2026, 6, 25, 15, 48, 28)),
        ];
        assert_eq!(
            best_folder_match(ca, None, &candidates),
            Some(folder),
            "should match the folder whose local start equals created_at-in-local"
        );
    }

    #[test]
    fn matches_within_tolerance_for_second_rounding() {
        let ca = utc(2026, 6, 25, 17, 6, 35);
        // Folder stamped a couple seconds earlier than the DB created_at.
        let drifted = utc(2026, 6, 25, 17, 6, 33);
        let folder = default_folder(drifted);
        let candidates = vec![folder.clone()];
        assert_eq!(best_folder_match(ca, None, &candidates), Some(folder));
    }

    #[test]
    fn rejects_when_outside_tolerance() {
        let ca = utc(2026, 6, 25, 17, 0, 47);
        // Five minutes off — a different meeting.
        let other = default_folder(utc(2026, 6, 25, 17, 5, 47));
        assert_eq!(best_folder_match(ca, None, &[other]), None);
    }

    #[test]
    fn returns_none_when_no_candidates() {
        let ca = utc(2026, 6, 25, 17, 0, 47);
        assert_eq!(best_folder_match(ca, None, &[]), None);
    }

    #[test]
    fn ignores_unparseable_folder_names() {
        let ca = utc(2026, 6, 25, 17, 0, 47);
        let candidates = vec![
            "not-a-meeting-folder".to_string(),
            ".DS_Store".to_string(),
            "models".to_string(),
        ];
        assert_eq!(best_folder_match(ca, None, &candidates), None);
    }

    #[test]
    fn title_corroboration_disambiguates_equally_close() {
        // Two folders with the SAME local start (a true timestamp tie); the title
        // breaks the tie toward the matching one.
        let ca = utc(2026, 6, 25, 17, 0, 47);
        let ls = local_start(ca);
        let um = utc_minute(ca);
        let titled = format!("UX_Product Sync_{}_{}", ls, um);
        let other = format!("Meeting {}_{}", ls, um);
        let candidates = vec![other.clone(), titled.clone()];
        assert_eq!(
            best_folder_match(ca, Some("UX/Product Sync"), &candidates),
            Some(titled),
            "title should restrict to the corroborating folder"
        );
    }

    #[test]
    fn ambiguous_timestamp_tie_without_title_returns_none() {
        let ca = utc(2026, 6, 25, 17, 0, 47);
        let ls = local_start(ca);
        let um = utc_minute(ca);
        // Two equally-close, equally-(non)corroborating folders → refuse to guess.
        let a = format!("Meeting {}_{}", ls, um);
        let b = format!("Standup {}_{}", ls, um);
        let candidates = vec![a, b];
        assert_eq!(best_folder_match(ca, None, &candidates), None);
    }

    #[test]
    fn closest_timestamp_wins_among_several() {
        let ca = utc(2026, 6, 25, 17, 0, 47);
        let exact = default_folder(ca);
        let near = default_folder(utc(2026, 6, 25, 17, 1, 30)); // 43s off, in tolerance
        let candidates = vec![near, exact.clone()];
        assert_eq!(best_folder_match(ca, None, &candidates), Some(exact));
    }
}
