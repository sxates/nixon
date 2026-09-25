//! Persisted on/off setting for speaker diarization (specs/0010, P1-B2).
//!
//! Stored as a small JSON file (`<app-data-dir>/diarization.json`) rooted at the
//! identifier-derived data dir so dev/prod stay isolated (ADR-0004) — mirroring
//! `zoom::settings`. Diarization is **off by default / opt-in** until accuracy is
//! validated on real calls (spec Decisions #2). The auto-run-after-transcription
//! hook (P1-C) gates on this value.

use anyhow::Result;
use log::info as log_info;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiarizationSettings {
    /// Run offline diarization (and auto-run it after a recording's transcription
    /// completes, P1-C). Off by default — opt-in.
    pub diarization_enabled: bool,
    /// Label speakers *live* while recording (specs/0011 P3-B). A separate sub-toggle
    /// under `diarization_enabled`: live diarization only ever runs when BOTH are on
    /// (and the models are present). Off by default — extra CPU, opt-in. Older
    /// settings files written before P3-B lack this field, so it `serde`-defaults to
    /// `false`, preserving v0.4.0 behaviour on upgrade.
    #[serde(default)]
    pub live_diarization_enabled: bool,
    /// Vestigial (specs/0066 W1). This was the "Expected number of speakers" override,
    /// a `Fixed(n)` tier that outranked every derived bound. The control is gone and
    /// nothing reads this any more — it stays only so an existing settings file still
    /// deserializes, and so a count someone set long ago cannot keep forcing clusters
    /// with no way to clear it. Sizing is the roster/calendar ceiling plus the
    /// audio-derived seed (specs/0050).
    #[serde(default)]
    pub expected_speaker_count: Option<u32>,
    /// The **one** voiceprint consent (specs/0078 owner decision 1; ADR-0007 §2/§3 as
    /// amended). Covers every durable voiceprint Nixon stores: other people's AND the
    /// device owner's ("You"). Off by default — storing anyone's biometric requires
    /// explicit consent. With this off, in-meeting suggestions and People mapping still
    /// work, but no `voiceprints` row is written, so cross-meeting voice recognition
    /// simply doesn't accrue. Per-person `voiceprint_opt_out` still applies on top for
    /// people other than the owner.
    ///
    /// Settings files written before specs/0078 carry this as `store_others_voiceprints`
    /// (read through the alias; the next save writes the new key). They may also carry
    /// the retired `self_enroll_voiceprint` owner toggle, which is ignored: serde skips
    /// unknown fields. Absent → `false`, the consent-required posture.
    #[serde(default, alias = "store_others_voiceprints")]
    pub store_voiceprints: bool,
    /// specs/0039 WS1 — optional override for the same-voice **consolidation**
    /// similarity floor (`sherpa::CONSOLIDATE_FLOOR`, default `0.70` since specs/0041
    /// WS1 — the shipped `0.50` fused distinct voices). The offline pass
    /// merges drift-split clusters of the *same* voice whose centroid cosine similarity
    /// is at or above this floor, repairing the "Speaker 1 early → someone else late"
    /// long-meeting symptom. `None` (the default) uses the built-in conservative const.
    /// Exposed purely for tuning on real long files; it must stay above the
    /// anti-false-merge `MERGE_FLOOR` (the diarizer clamps any override). Older settings
    /// files lack this field, so it `serde`-defaults to `None`, preserving 1.6 behaviour.
    #[serde(default)]
    pub consolidation_floor: Option<f32>,
}

/// How the diarization speaker count was decided, surfaced to the frontend on the
/// `diarization-complete` event so the meeting view can show the basis (e.g.
/// "Estimated 9 speakers from calendar"). Serialized as the lowercase variant name
/// (`"calendar"|"auto"`) on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SpeakerCountSource {
    /// Seeded from the linked calendar event's remote attendees (Fixed, estimate).
    Calendar,
    /// Auto-clustering with the specs/0050 audio-derived cap (`AtMost(n_audio)`),
    /// applied in the diarization pass. Produced for ad-hoc meetings and ones with a
    /// distribution-list invite (no reliable ceiling). Superseded specs/0048's fixed
    /// default `AtMost` cap.
    Auto,
}

impl SpeakerCountSource {
    /// The wire string for the `speakerCountSource` IPC field.
    pub fn as_str(self) -> &'static str {
        match self {
            SpeakerCountSource::Calendar => "calendar",
            SpeakerCountSource::Auto => "auto",
        }
    }
}

/// Resolve the diarization speaker count by precedence, returning both the
/// [`SpeakerCount`] mode and *how* it was chosen ([`SpeakerCountSource`]).
///
/// Precedence (specs/0011 calendar-seed; specs/0017 count-as-MAX). The manual
/// "Expected number of speakers" override used to sit above all of this and force
/// `Fixed(n)`; specs/0066 W1 removed it, so the audio- and roster-derived bounds are
/// now the whole story:
/// 1. **Calendar/roster-seed** — count the *remote* attendees in
///    `calendar_attendees` (everyone EXCLUDING the current user, since diarization
///    only clusters the system channel; the local user is the mic channel and is
///    assigned separately). If `remote_count >= 1` → `AtMost(remote_count)` /
///    [`SpeakerCountSource::Calendar`].
///    NOTE (specs/0017): the attendee/roster count is an UPPER BOUND, not a target
///    — not everyone invited speaks. `AtMost(n)` runs Auto clustering and then
///    merges down to *at most* `n` clusters, so a 10-invited / 4-spoke meeting is
///    never force-split to 10; it still caps unbounded Auto over-counting on long
///    calls (e.g. a 90-min/~10-person call auto-clustered to 27).
/// 2. **Else** (ad-hoc, or a distribution-list invite with no reliable ceiling) —
///    `Auto` / [`SpeakerCountSource::Auto`]. specs/0050's audio seed then caps it at
///    `AtMost(n_audio)` in the diarization pass (superseding specs/0048's fixed default
///    cap, which would wrongly cap a genuine large ad-hoc meeting below the audio count).
///
/// Pure: the EventKit lookup happens at the call site and is passed in here, so
/// the precedence is independently unit-testable. `calendar_attendees` should be
/// the already-resolved attendees of the meeting's linked event (empty when the
/// lookup failed, access was denied, or no event matched — all of which fall
/// through to Auto).
pub fn resolve_speaker_count(
    calendar_attendees: &[crate::calendar::eventkit::Attendee],
) -> (crate::diarization::SpeakerCount, SpeakerCountSource) {
    use crate::diarization::SpeakerCount;

    // 1. Calendar/roster-seed from remote attendees — a CLEAN invite only (specs/0050).
    //    A distribution-list invite is one entry for many people, so the count is
    //    unreliable and would produce a too-low `AtMost` that force-merges real speakers;
    //    when a DL is present we fall through to Auto and let the audio seed
    //    (`estimate_speakers_by_duration`) decide. Otherwise the exclude-self remote
    //    count is an UPPER BOUND (specs/0017) — a ceiling the audio seed caps against.
    let has_distribution_list = calendar_attendees.iter().any(|a| a.is_distribution_list);
    let remote_count = calendar_attendees
        .iter()
        .filter(|a| !a.is_current_user && !a.is_distribution_list)
        .count();
    if !has_distribution_list && remote_count >= 1 {
        let n = remote_count.min(u32::MAX as usize) as u32;
        return (SpeakerCount::AtMost(n), SpeakerCountSource::Calendar);
    }

    // 3. Nothing reliable about the meeting size (no manual, no clean roster/calendar,
    //    or a distribution-list invite) → Auto. specs/0050's audio seed caps Auto at
    //    `AtMost(n_audio)` in the diarization pass, superseding specs/0048's fixed default
    //    cap (which would wrongly cap a genuine large ad-hoc meeting below n_audio).
    (SpeakerCount::Auto, SpeakerCountSource::Auto)
}

// `false` happens to equal the derived bool default, but we spell it out: the
// off-by-default / opt-in behaviour is a deliberate product decision (spec
// Decisions #2; specs/0011 for the live sub-toggle), not an accident of the type.
#[allow(clippy::derivable_impls)]
impl Default for DiarizationSettings {
    fn default() -> Self {
        Self {
            // Opt-in / off by default (spec Decisions #2).
            diarization_enabled: false,
            // Live labels off by default (specs/0011 P3-B): extra CPU, opt-in.
            live_diarization_enabled: false,
            // Auto cluster count by default (no override); user can set an exact
            // expected count to force Fixed mode (specs/0011 accuracy gate).
            expected_speaker_count: None,
            // Storing any voiceprint, the owner's included, is opt-in / off by default
            // (specs/0078 owner decision 1; ADR-0007 §2).
            store_voiceprints: false,
            // No consolidation-floor override by default: use the built-in
            // conservative const (specs/0039 WS1). Set only for real-file tuning.
            consolidation_floor: None,
        }
    }
}

/// Path of the diarization settings JSON file (`<app-data-dir>/diarization.json`).
fn settings_path() -> Result<PathBuf> {
    let path = crate::app_paths::app_data_dir().join("diarization.json");
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    Ok(path)
}

/// Load diarization settings from disk, falling back to defaults if absent/unreadable.
pub async fn load_settings() -> DiarizationSettings {
    let path = match settings_path() {
        Ok(p) => p,
        Err(e) => {
            log_info!(
                "Could not resolve diarization settings path ({}), using defaults",
                e
            );
            return DiarizationSettings::default();
        }
    };

    if !path.exists() {
        return DiarizationSettings::default();
    }

    match tokio::fs::read_to_string(&path).await {
        Ok(content) => serde_json::from_str(&content).unwrap_or_else(|e| {
            log_info!(
                "Failed to parse diarization settings ({}), using defaults",
                e
            );
            DiarizationSettings::default()
        }),
        Err(e) => {
            log_info!(
                "Failed to read diarization settings ({}), using defaults",
                e
            );
            DiarizationSettings::default()
        }
    }
}

/// Persist diarization settings to disk.
pub async fn save_settings(settings: &DiarizationSettings) -> Result<()> {
    let path = settings_path()?;
    let content = serde_json::to_string_pretty(settings)?;
    tokio::fs::write(&path, content).await?;
    log_info!(
        "Saved diarization settings (diarization_enabled={}, live_diarization_enabled={}, expected_speaker_count={:?}, store_voiceprints={})",
        settings.diarization_enabled,
        settings.live_diarization_enabled,
        settings.expected_speaker_count,
        settings.store_voiceprints
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diarization::SpeakerCount;

    use crate::calendar::eventkit::Attendee;

    /// Build an attendee for the precedence tests.
    fn attendee(is_current_user: bool) -> Attendee {
        Attendee {
            name: if is_current_user {
                "Me".into()
            } else {
                "Someone".into()
            },
            email: None,
            is_current_user,
            is_distribution_list: false,
            photo_data_uri: None,
        }
    }

    /// specs/0066 W1: a settings file written while the "Expected number of speakers"
    /// control existed still carries the count, and it must be inert — a value someone
    /// set once, with the control now gone, would otherwise force clusters forever with
    /// no way to clear it. The resolver no longer takes the settings at all, so the
    /// compiler enforces this; the test pins the round-trip plus the resulting sizing.
    #[test]
    fn a_leftover_expected_count_no_longer_forces_anything() {
        let json = r#"{"diarization_enabled":true,"live_diarization_enabled":false,"expected_speaker_count":3}"#;
        let s: DiarizationSettings = serde_json::from_str(json).expect("parse settings");
        assert_eq!(s.expected_speaker_count, Some(3), "the field still parses");

        // Two remote attendees seed the ceiling; the leftover 3 cannot reach the decision.
        let attendees = vec![attendee(false), attendee(false), attendee(true)];
        let (count, source) = resolve_speaker_count(&attendees);
        assert!(matches!(count, SpeakerCount::AtMost(2)));
        assert_eq!(source, SpeakerCountSource::Calendar);
    }

    #[test]
    fn calendar_seeds_remote_count_excluding_self() {
        // 10 total attendees including self → 9 remote → AtMost(9) (specs/0017: cap,
        // not a forced count — not everyone invited speaks).
        let mut attendees = vec![attendee(true)]; // the current user
        attendees.extend(std::iter::repeat_with(|| attendee(false)).take(9));
        let (count, source) = resolve_speaker_count(&attendees);
        assert!(matches!(count, SpeakerCount::AtMost(9)));
        assert_eq!(source, SpeakerCountSource::Calendar);
    }

    #[test]
    fn one_remote_attendee_caps_at_one() {
        let attendees = vec![attendee(false), attendee(true)];
        let (count, source) = resolve_speaker_count(&attendees);
        // specs/0017: calendar branch is now an upper bound.
        assert!(matches!(count, SpeakerCount::AtMost(1)));
        assert_eq!(source, SpeakerCountSource::Calendar);
    }

    #[test]
    fn no_remote_attendees_is_auto() {
        // specs/0050: an unknown-size meeting (only the current user as attendee →
        // 0 remote) resolves to Auto; the audio seed caps it at AtMost(n_audio) in the
        // diarization pass (reverts specs/0048's harmful fixed default cap).
        let (count, source) = resolve_speaker_count(&[attendee(true)]);
        assert!(matches!(count, SpeakerCount::Auto));
        assert_eq!(source, SpeakerCountSource::Auto);
    }

    #[test]
    fn an_empty_calendar_is_auto() {
        // specs/0050: ad-hoc meeting (no roster, no linked calendar event) → Auto.
        let (count, source) = resolve_speaker_count(&[]);
        assert!(matches!(count, SpeakerCount::Auto));
        assert_eq!(source, SpeakerCountSource::Auto);
    }

    #[test]
    fn distribution_list_invite_falls_through_to_auto() {
        // specs/0050: a distribution-list invite under-counts (1 entry = many people),
        // so we must NOT produce a tight AtMost that would force-merge real speakers —
        // fall through to Auto and let the audio seed decide.
        let dl = Attendee {
            is_distribution_list: true,
            ..attendee(false)
        };
        let attendees = vec![attendee(true), dl, attendee(false)];
        let (count, source) = resolve_speaker_count(&attendees);
        assert!(matches!(count, SpeakerCount::Auto));
        assert_eq!(source, SpeakerCountSource::Auto);
    }

    #[test]
    fn source_wire_strings_are_stable() {
        assert_eq!(SpeakerCountSource::Calendar.as_str(), "calendar");
        assert_eq!(SpeakerCountSource::Auto.as_str(), "auto");
    }

    #[test]
    fn legacy_settings_without_expected_count_deserialize_to_auto() {
        // Older settings files (pre-0011) lack the field; it must default to None.
        let json = r#"{"diarization_enabled":true,"live_diarization_enabled":false}"#;
        let s: DiarizationSettings = serde_json::from_str(json).expect("parse legacy settings");
        assert_eq!(s.expected_speaker_count, None);
        let (count, source) = resolve_speaker_count(&[]);
        assert!(matches!(count, SpeakerCount::Auto));
        assert_eq!(source, SpeakerCountSource::Auto);
    }

    #[test]
    fn legacy_settings_default_voiceprint_consent_conservatively() {
        // A settings file written before 1c lacks the voiceprint fields. Storing
        // voiceprints must default OFF (consent-required, ADR-0007 §2 as amended by
        // specs/0078: one consent covers the owner too).
        let json = r#"{"diarization_enabled":true,"live_diarization_enabled":false}"#;
        let s: DiarizationSettings = serde_json::from_str(json).expect("parse legacy settings");
        assert!(!s.store_voiceprints, "voiceprint storage must default off");
    }

    /// specs/0078: a settings file from before the rename still loads. The old
    /// `store_others_voiceprints` key carries the consent across, and the retired
    /// `self_enroll_voiceprint` key is ignored rather than failing the parse (a failed
    /// parse would silently reset every diarization setting to its default).
    #[test]
    fn pre_0078_voiceprint_keys_still_load() {
        let json = r#"{"diarization_enabled":true,"live_diarization_enabled":true,
            "store_others_voiceprints":true,"self_enroll_voiceprint":false}"#;
        let s: DiarizationSettings = serde_json::from_str(json).expect("parse pre-0078 settings");
        assert!(s.store_voiceprints, "the old key is read through the alias");
        assert!(
            s.live_diarization_enabled,
            "the rest of the file is not reset"
        );

        // The next save writes the new key only.
        let written = serde_json::to_string(&s).unwrap();
        assert!(written.contains("\"store_voiceprints\":true"), "{written}");
        assert!(!written.contains("store_others_voiceprints"), "{written}");
        assert!(!written.contains("self_enroll_voiceprint"), "{written}");
    }

    #[test]
    fn legacy_settings_without_consolidation_floor_default_to_none() {
        // A settings file written before specs/0039 lacks the field; it must default
        // to None (use the built-in CONSOLIDATE_FLOOR), preserving 1.6 behaviour.
        let json = r#"{"diarization_enabled":true,"live_diarization_enabled":false}"#;
        let s: DiarizationSettings = serde_json::from_str(json).expect("parse legacy settings");
        assert_eq!(s.consolidation_floor, None);
    }
}
