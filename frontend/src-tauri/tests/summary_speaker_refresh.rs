//! specs/0041 WS2 — post-diarization summary-refresh trigger, DB round-trip.
//!
//! The pure decision matrix is unit-tested in `summary::refresh`; these tests prove the
//! decision inputs survive the REAL migration + repository path: the 0041 migration adds
//! `speaker_attributed` / `generated_markdown_hash` to `summary_processes`,
//! `update_process_completed` persists them, `load_snapshot` reads them back, and the
//! user-edit path (`update_meeting_summary`, which rewrites `result` wholesale) flips
//! the pristine guard without touching the stored fingerprint.

mod common;

use app_lib::database::repositories::meeting::MeetingsRepository;
use app_lib::database::repositories::summary::SummaryProcessesRepository;
use app_lib::summary::refresh::{
    decide_speaker_refresh, generation_fingerprint, load_snapshot, SpeakerRefreshDecision,
};

const MARKDOWN: &str = "## Decisions\nShip the roadmap\n## Action Items\n- send the deck";

/// Creates a meeting and completes a summary process for it, persisting the given
/// speaker-attribution flag and the generation-time fingerprint of `MARKDOWN`.
async fn seed_completed_summary(pool: &sqlx::SqlitePool, speaker_attributed: bool) -> String {
    let meeting_id = MeetingsRepository::create_meeting(pool, None, None, None, None, None)
        .await
        .expect("create_meeting");

    SummaryProcessesRepository::create_or_reset_process(pool, &meeting_id)
        .await
        .expect("create_or_reset_process");

    SummaryProcessesRepository::update_process_completed(
        pool,
        &meeting_id,
        serde_json::json!({ "markdown": MARKDOWN }),
        1,
        1.5,
        speaker_attributed,
        &generation_fingerprint(MARKDOWN),
        &app_lib::summary::refresh::speaker_names_fingerprint(std::iter::empty()),
    )
    .await
    .expect("update_process_completed");

    meeting_id
}

/// The headline case: a speakerless, pristine, completed summary + a diarization pass
/// that assigned speakers → the trigger decides Refresh.
#[tokio::test]
async fn speakerless_pristine_summary_decides_refresh_after_db_roundtrip() {
    let (_dir, db) = common::fresh_db().await;
    let pool = db.pool();

    let meeting_id = seed_completed_summary(pool, false).await;

    let snapshot = load_snapshot(pool, &meeting_id)
        .await
        .expect("load_snapshot")
        .expect("summary process row exists");
    assert!(!snapshot.speaker_attributed);
    assert_eq!(snapshot.current_markdown.as_deref(), Some(MARKDOWN));

    assert_eq!(
        decide_speaker_refresh(2, Some(&snapshot)),
        SpeakerRefreshDecision::Refresh
    );
}

/// Loop guard: the regenerated summary persists `speaker_attributed = 1`, so a second
/// diarization pass must NOT re-run it.
#[tokio::test]
async fn attributed_summary_is_not_rerun() {
    let (_dir, db) = common::fresh_db().await;
    let pool = db.pool();

    let meeting_id = seed_completed_summary(pool, true).await;

    let snapshot = load_snapshot(pool, &meeting_id)
        .await
        .expect("load_snapshot")
        .expect("summary process row exists");

    assert_eq!(
        decide_speaker_refresh(2, Some(&snapshot)),
        SpeakerRefreshDecision::SkipAlreadyAttributed
    );
}

/// Pristine guard: a user edit goes through `update_meeting_summary`, which rewrites
/// `result` but leaves the generation-time fingerprint alone — the mismatch must block
/// the auto-regenerate so user edits are never clobbered.
#[tokio::test]
async fn user_edited_summary_is_not_rerun() {
    let (_dir, db) = common::fresh_db().await;
    let pool = db.pool();

    let meeting_id = seed_completed_summary(pool, false).await;

    // The user edits the summary in the BlockNote editor and saves.
    let edited = SummaryProcessesRepository::update_meeting_summary(
        pool,
        &meeting_id,
        &serde_json::json!({ "markdown": format!("{MARKDOWN}\n\nmy own appended notes") }),
    )
    .await
    .expect("update_meeting_summary");
    assert!(edited, "edit should find the meeting");

    let snapshot = load_snapshot(pool, &meeting_id)
        .await
        .expect("load_snapshot")
        .expect("summary process row exists");

    assert_eq!(
        decide_speaker_refresh(2, Some(&snapshot)),
        SpeakerRefreshDecision::SkipUserEdited
    );
}

/// A diarization pass that assigned no speakers changes nothing — never re-run.
#[tokio::test]
async fn no_speakers_assigned_is_not_rerun() {
    let (_dir, db) = common::fresh_db().await;
    let pool = db.pool();

    let meeting_id = seed_completed_summary(pool, false).await;
    let snapshot = load_snapshot(pool, &meeting_id)
        .await
        .expect("load_snapshot");

    assert_eq!(
        decide_speaker_refresh(0, snapshot.as_ref()),
        SpeakerRefreshDecision::SkipNoSpeakersAssigned
    );
}

/// Legacy rows completed before the 0041 migration have no fingerprint: the migration's
/// defaults (`speaker_attributed = 0`, `generated_markdown_hash IS NULL`) must land on
/// the conservative side — treated as unverifiable, so no auto-regenerate.
#[tokio::test]
async fn legacy_row_without_fingerprint_is_not_rerun() {
    let (_dir, db) = common::fresh_db().await;
    let pool = db.pool();

    let meeting_id = seed_completed_summary(pool, false).await;
    sqlx::query("UPDATE summary_processes SET generated_markdown_hash = NULL WHERE meeting_id = ?")
        .bind(&meeting_id)
        .execute(pool)
        .await
        .expect("simulate pre-0041 row");

    let snapshot = load_snapshot(pool, &meeting_id)
        .await
        .expect("load_snapshot")
        .expect("summary process row exists");

    assert_eq!(
        decide_speaker_refresh(2, Some(&snapshot)),
        SpeakerRefreshDecision::SkipUserEdited
    );
}

/// specs/0044 WS3 — the names-hash round-trip: the 0044 migration adds
/// `speaker_names_hash`, `update_process_completed` persists it, `load_snapshot`
/// reads it back, and `decide_name_refresh` flips from SkipNamesUnchanged to
/// Refresh exactly when the CURRENT resolved name set diverges from it.
#[tokio::test]
async fn naming_change_flips_name_refresh_decision_after_db_roundtrip() {
    use app_lib::summary::refresh::{
        current_speaker_names_hash, decide_name_refresh, speaker_names_fingerprint,
    };

    let (_dir, db) = common::fresh_db().await;
    let pool = db.pool();

    let meeting_id = MeetingsRepository::create_meeting(pool, None, None, None, None, None)
        .await
        .expect("create_meeting");
    // One diarized segment resolving to the placeholder name "Speaker 1".
    sqlx::query(
        "INSERT INTO transcripts (id, meeting_id, transcript, timestamp, audio_start_time, audio_end_time, speaker)
         VALUES ('t-1', ?, 'hello there', '2026-08-02T00:00:00Z', 0.0, 2.0, 'spk_0')",
    )
    .bind(&meeting_id)
    .execute(pool)
    .await
    .expect("insert transcript");
    sqlx::query(
        "INSERT INTO speakers (id, meeting_id, speaker_key, display_name, created_at, updated_at)
         VALUES ('s-1', ?, 'spk_0', 'Speaker 1', '2026-08-02T00:00:00Z', '2026-08-02T00:00:00Z')",
    )
    .bind(&meeting_id)
    .execute(pool)
    .await
    .expect("insert speaker");

    // Summary generated while the speaker was still the placeholder — even
    // speaker-attributed (the 0041 trigger would skip this row).
    SummaryProcessesRepository::create_or_reset_process(pool, &meeting_id)
        .await
        .expect("create_or_reset_process");
    SummaryProcessesRepository::update_process_completed(
        pool,
        &meeting_id,
        serde_json::json!({ "markdown": MARKDOWN }),
        1,
        1.5,
        true,
        &generation_fingerprint(MARKDOWN),
        &speaker_names_fingerprint(["Speaker 1"]),
    )
    .await
    .expect("update_process_completed");

    // Same names → no-op.
    let current = current_speaker_names_hash(pool, &meeting_id)
        .await
        .expect("current names hash");
    let snapshot = load_snapshot(pool, &meeting_id)
        .await
        .expect("load_snapshot")
        .expect("row exists");
    assert_eq!(
        decide_name_refresh(&current, Some(&snapshot)),
        SpeakerRefreshDecision::SkipNamesUnchanged
    );

    // The user names the speaker → the current set diverges → Refresh, even though
    // speaker_attributed = 1 (the 0044 relaxation over decide_speaker_refresh).
    sqlx::query("UPDATE speakers SET display_name = 'Priya' WHERE id = 's-1'")
        .execute(pool)
        .await
        .expect("rename speaker");
    let current = current_speaker_names_hash(pool, &meeting_id)
        .await
        .expect("current names hash");
    assert_eq!(
        decide_name_refresh(&current, Some(&snapshot)),
        SpeakerRefreshDecision::Refresh
    );
    assert_eq!(
        decide_speaker_refresh(2, Some(&snapshot)),
        SpeakerRefreshDecision::SkipAlreadyAttributed,
        "the 0041 trigger alone would never pick up the rename"
    );
}

/// An in-flight run (create_or_reset stores uppercase 'PENDING') defers the decision.
#[tokio::test]
async fn in_flight_summary_run_defers() {
    let (_dir, db) = common::fresh_db().await;
    let pool = db.pool();

    let meeting_id = MeetingsRepository::create_meeting(pool, None, None, None, None, None)
        .await
        .expect("create_meeting");
    SummaryProcessesRepository::create_or_reset_process(pool, &meeting_id)
        .await
        .expect("create_or_reset_process");

    let snapshot = load_snapshot(pool, &meeting_id)
        .await
        .expect("load_snapshot")
        .expect("summary process row exists");

    assert_eq!(
        decide_speaker_refresh(2, Some(&snapshot)),
        SpeakerRefreshDecision::SkipSummaryInFlight
    );
}
