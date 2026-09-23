use super::*;
use chrono::TimeZone;

fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 9, 22, 12, 0, 0).unwrap()
}

fn facts(state: AudioState, age_days: i64) -> MeetingAudioFacts {
    MeetingAudioFacts {
        state,
        created_at: now() - Duration::days(age_days) - Duration::seconds(1),
        folder_recording: false,
        awaiting_transcription: false,
        has_wav_channels: true,
    }
}

const AFTER: AudioRetention = AudioRetention::AfterProcessing;
const DAYS7: AudioRetention = AudioRetention::Days { days: 7 };
const FOREVER: AudioRetention = AudioRetention::Forever;
const ALL: [AudioRetention; 3] = [AFTER, DAYS7, FOREVER];

// --- row 1: pending / recording / awaiting transcription → Keep under every policy ---

#[test]
fn a_pending_meeting_is_never_deleted_whatever_its_age() {
    for policy in ALL {
        assert_eq!(
            disposition(policy, &facts(AudioState::Pending, 10_000), now()),
            Disposition::Keep(KeepReason::Processing),
            "{policy:?}"
        );
    }
}

#[test]
fn a_folder_still_recording_is_kept_in_every_state() {
    for policy in ALL {
        for state in [
            AudioState::Pending,
            AudioState::Processed,
            AudioState::Failed,
        ] {
            let mut m = facts(state, 400);
            m.folder_recording = true;
            assert_eq!(
                disposition(policy, &m, now()),
                Disposition::Keep(KeepReason::Recording),
                "{policy:?} {state:?}"
            );
        }
    }
}

/// Sabotage target (spec #1): a deferred meeting is never deleted.
#[test]
fn a_meeting_awaiting_transcription_is_kept_even_when_processed_and_old() {
    for policy in ALL {
        for state in [
            AudioState::Pending,
            AudioState::Processed,
            AudioState::Failed,
        ] {
            let mut m = facts(state, 400);
            m.awaiting_transcription = true;
            assert_eq!(
                disposition(policy, &m, now()),
                Disposition::Keep(KeepReason::AwaitingProcessing),
                "{policy:?} {state:?}"
            );
        }
    }
}

// --- row 2: processed ---

#[test]
fn processed_under_once_processed_is_deleted_at_any_age() {
    assert_eq!(
        disposition(AFTER, &facts(AudioState::Processed, 0), now()),
        Disposition::Delete
    );
}

#[test]
fn processed_under_days_is_deleted_only_when_strictly_older() {
    assert_eq!(
        disposition(DAYS7, &facts(AudioState::Processed, 7), now()),
        Disposition::Delete,
        "7 days + 1 s is past the window"
    );
    let mut exactly = facts(AudioState::Processed, 0);
    exactly.created_at = now() - Duration::days(7);
    assert_eq!(
        disposition(DAYS7, &exactly, now()),
        Disposition::Compress,
        "exactly 7 days old is kept (strict >)"
    );
}

#[test]
fn processed_and_kept_compresses_wav_channels_once() {
    for policy in [DAYS7, FOREVER] {
        let mut m = facts(AudioState::Processed, 1);
        assert_eq!(disposition(policy, &m, now()), Disposition::Compress);
        m.has_wav_channels = false;
        assert_eq!(
            disposition(policy, &m, now()),
            Disposition::Keep(KeepReason::Policy),
            "already compressed: nothing to do"
        );
    }
    assert_eq!(
        disposition(FOREVER, &facts(AudioState::Processed, 10_000), now()),
        Disposition::Compress,
        "Forever never deletes"
    );
}

// --- row 3: failed ---

#[test]
fn failed_under_once_processed_survives_the_grace_period_then_goes() {
    assert_eq!(
        disposition(AFTER, &facts(AudioState::Failed, 6), now()),
        Disposition::Keep(KeepReason::Failed)
    );
    assert_eq!(
        disposition(
            AFTER,
            &facts(AudioState::Failed, i64::from(FAILED_AUDIO_GRACE_DAYS)),
            now()
        ),
        Disposition::Delete
    );
}

#[test]
fn failed_under_days_follows_the_day_count_and_is_never_compressed() {
    let days30 = AudioRetention::Days { days: 30 };
    assert_eq!(
        disposition(days30, &facts(AudioState::Failed, 10), now()),
        Disposition::Keep(KeepReason::Failed),
        "past the 7-day grace, inside 30 days: kept"
    );
    assert_eq!(
        disposition(days30, &facts(AudioState::Failed, 30), now()),
        Disposition::Delete
    );
}

#[test]
fn failed_under_forever_is_kept() {
    assert_eq!(
        disposition(FOREVER, &facts(AudioState::Failed, 10_000), now()),
        Disposition::Keep(KeepReason::Failed)
    );
}

// --- row 4: purged ---

#[test]
fn purged_is_a_no_op_under_every_policy() {
    for policy in ALL {
        assert_eq!(
            disposition(policy, &facts(AudioState::Purged, 10_000), now()),
            Disposition::Keep(KeepReason::AlreadyPurged)
        );
    }
}

// --- state round trip ---

#[test]
fn audio_state_round_trips_through_the_column() {
    for state in [
        AudioState::Pending,
        AudioState::Processed,
        AudioState::Failed,
        AudioState::Purged,
    ] {
        assert_eq!(AudioState::from_db(state.as_db()), state);
    }
    assert_eq!(AudioState::Pending.as_str(), "pending");
}

// --- preferences: legacy derivation (task 6) ---

/// Sabotage target (spec #5).
#[test]
fn legacy_immediately_is_once_processed_even_with_a_stale_day_count() {
    assert_eq!(AudioRetention::from_legacy(false, None), AFTER);
    assert_eq!(AudioRetention::from_legacy(false, Some(30)), AFTER);
}

#[test]
fn legacy_day_count_and_keep_forever_map_across() {
    assert_eq!(
        AudioRetention::from_legacy(true, Some(30)),
        AudioRetention::Days { days: 30 }
    );
    assert_eq!(AudioRetention::from_legacy(true, None), FOREVER);
    assert_eq!(AudioRetention::from_legacy(true, Some(0)), FOREVER);
}

#[test]
fn the_serialized_shape_is_the_documented_one() {
    let json = |p: AudioRetention| serde_json::to_string(&p).unwrap();
    assert_eq!(json(AFTER), r#"{"mode":"after_processing"}"#);
    assert_eq!(json(DAYS7), r#"{"mode":"days","days":7}"#);
    assert_eq!(json(FOREVER), r#"{"mode":"forever"}"#);
    let parsed: AudioRetention = serde_json::from_str(r#"{"mode":"days","days":30}"#).unwrap();
    assert_eq!(parsed, AudioRetention::Days { days: 30 });
}

#[test]
fn saving_prefers_whichever_retention_field_the_sender_changed() {
    let stored = Some(DAYS7);
    let stored_legacy = (true, Some(7));
    // A new settings screen changed the policy.
    assert_eq!(
        reconcile_saved_retention(stored, stored_legacy, Some(FOREVER), stored_legacy),
        FOREVER
    );
    // An older screen round-tripped the stored policy but changed the legacy pair.
    assert_eq!(
        reconcile_saved_retention(stored, stored_legacy, stored, (false, Some(7))),
        AFTER
    );
    // An unrelated save (no retention change) keeps what was stored.
    assert_eq!(
        reconcile_saved_retention(stored, stored_legacy, None, stored_legacy),
        DAYS7
    );
    // A file from before 0072 derives from the legacy pair.
    assert_eq!(
        reconcile_saved_retention(None, (false, None), None, (false, None)),
        AFTER
    );
}
