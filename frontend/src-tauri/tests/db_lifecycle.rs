//! Persist-at-start lifecycle test (specs/0009, priority #4; covers specs/0007).
//!
//! Exercises the DB lifecycle the recording flow relies on, with NO audio and NO
//! Tauri runtime. It uses a temp-file SQLite database brought up through the
//! app's real migration path (`DatabaseManager::new`), then drives the same
//! repository calls the `api_create_meeting` / save / `api_delete_meeting`
//! commands wrap:
//!
//!   create_meeting (at recording START)
//!     -> save_transcripts_for_meeting (at STOP)
//!     -> get_meetings_enriched (dashboard)
//!     -> delete_meeting (cleanup)
//!
//! We test the repository layer rather than the Tauri command layer because the
//! commands require an `AppHandle`/`State` that cannot be constructed without a
//! running app; the repositories contain the actual SQL/logic under test.
//!
//! Run with:
//!   cargo test --features metal --test db_lifecycle -- --nocapture

mod common;

use app_lib::database::repositories::meeting::MeetingsRepository;
use app_lib::database::repositories::transcript::TranscriptsRepository;
use app_lib::meetings::recording_is_safe_to_discard;
use common::{fresh_db, segment};

#[tokio::test]
async fn persist_at_start_lifecycle() {
    let (_dir, db) = fresh_db().await;
    let pool = db.pool();

    // 1. START: create an empty meeting (notes autosave against this id mid-call).
    let meeting_id = MeetingsRepository::create_meeting(pool, None, None, None, None, None)
        .await
        .expect("create_meeting");

    // It exists, is the only one, has the placeholder title, and no duration yet.
    let after_create = MeetingsRepository::get_meetings_enriched(pool)
        .await
        .expect("enriched after create");
    assert_eq!(
        after_create.len(),
        1,
        "expected exactly one meeting after create"
    );
    assert_eq!(after_create[0].id, meeting_id);
    assert_eq!(after_create[0].title, "New Meeting");
    assert!(
        after_create[0].duration_seconds.is_none(),
        "no transcripts yet -> duration should be NULL"
    );

    // 2. STOP: attach transcripts + a real title to the SAME meeting.
    let segments = vec![
        segment("the quick brown fox", 0.0, 2.0),
        segment("jumps over the lazy dog", 2.0, 5.5),
    ];
    let attached = TranscriptsRepository::save_transcripts_for_meeting(
        pool,
        &meeting_id,
        "Standup 2026-06-24",
        &segments,
        None,
    )
    .await
    .expect("save_transcripts_for_meeting");
    assert!(
        attached,
        "expected save to find and update the existing meeting"
    );

    // 3. DASHBOARD: still ONE meeting, now enriched with title/duration/gist.
    let enriched = MeetingsRepository::get_meetings_enriched(pool)
        .await
        .expect("enriched after save");
    assert_eq!(enriched.len(), 1, "save must not create a second meeting");
    let row = &enriched[0];
    assert_eq!(row.id, meeting_id);
    assert_eq!(row.title, "Standup 2026-06-24");
    assert_eq!(
        row.duration_seconds,
        Some(5.5),
        "durationSeconds should be MAX(audio_end_time)"
    );
    // Gist fallback (no summary yet) = earliest transcript line.
    assert_eq!(
        row.first_transcript.as_deref(),
        Some("the quick brown fox"),
        "first_transcript gist should be the earliest segment"
    );

    // The transcripts are actually attached and ordered.
    let details = MeetingsRepository::get_meeting(pool, &meeting_id)
        .await
        .expect("get_meeting")
        .expect("meeting should exist");
    assert_eq!(details.transcripts.len(), 2, "both segments attached");
    let full = TranscriptsRepository::get_full_transcript(pool, &meeting_id)
        .await
        .expect("get_full_transcript");
    assert_eq!(full, "the quick brown fox\njumps over the lazy dog");

    // 4. CLEANUP: delete removes the meeting and its transcripts.
    let deleted = MeetingsRepository::delete_meeting(pool, &meeting_id)
        .await
        .expect("delete_meeting");
    assert!(deleted, "delete_meeting should report success");

    let after_delete = MeetingsRepository::get_meetings_enriched(pool)
        .await
        .expect("enriched after delete");
    assert!(
        after_delete.is_empty(),
        "no meetings should remain after delete"
    );

    let orphan_transcript = TranscriptsRepository::get_full_transcript(pool, &meeting_id)
        .await
        .expect("get_full_transcript after delete");
    assert!(
        orphan_transcript.is_empty(),
        "transcripts should be removed with the meeting (no orphans)"
    );
}

/// Saving against a non-existent meeting id must NOT create one — it signals the
/// caller (returns false) so the command layer can fall back to creating a
/// meeting. Guards the persist-at-start contract.
#[tokio::test]
async fn save_to_missing_meeting_returns_false() {
    let (_dir, db) = fresh_db().await;
    let pool = db.pool();

    let attached = TranscriptsRepository::save_transcripts_for_meeting(
        pool,
        "meeting-does-not-exist",
        "Ghost",
        &[segment("hello", 0.0, 1.0)],
        None,
    )
    .await
    .expect("save call ok");
    assert!(!attached, "saving to a missing meeting should return false");

    let meetings = MeetingsRepository::get_meetings_enriched(pool)
        .await
        .expect("enriched");
    assert!(meetings.is_empty(), "no meeting should have been created");
}

/// specs/0019 WS6.7 regression — a second recording session that hands a STALE
/// meeting_id (a prior, calendar-linked meeting that already holds transcripts) must
/// NOT be merged into it. This reproduces all three reported symptoms in one place:
/// interwoven transcripts, an overwritten folder_path, and a preserved (now-wrong)
/// calendar_event_id driving the wrong attendee roster. The save must refuse so the
/// caller mints a fresh row, leaving the original meeting completely untouched.
#[tokio::test]
async fn second_session_does_not_merge_into_populated_meeting() {
    let (_dir, db) = fresh_db().await;
    let pool = db.pool();

    // Session A: a Join & Record meeting — title + calendar_event_id + its own folder.
    let meeting_a = MeetingsRepository::create_meeting(
        pool,
        Some("Weekly Sync".into()),
        Some("/recordings/weekly-sync_A".into()),
        Some("recorded".into()),
        Some("evt-AAA".into()),
        None,
    )
    .await
    .expect("create meeting A");

    let session_a = vec![
        segment("alice kicks off the sync", 0.0, 2.0),
        segment("bob shares an update", 2.0, 4.0),
    ];
    let attached_a = TranscriptsRepository::save_transcripts_for_meeting(
        pool,
        &meeting_a,
        "Weekly Sync",
        &session_a,
        Some("/recordings/weekly-sync_A".into()),
    )
    .await
    .expect("save session A");
    assert!(attached_a, "session A attaches to its own empty row");

    // Session B: the bug trigger — a NEW recording reuses meeting A's id at stop, with
    // a generic date-stamp title and session B's own folder + (different) transcripts.
    let session_b = vec![
        segment("totally unrelated standup", 0.0, 2.0),
        segment("different people talking", 2.0, 4.0),
    ];
    let attached_b = TranscriptsRepository::save_transcripts_for_meeting(
        pool,
        &meeting_a,
        "Meeting 2026-06-29_14-30-00",
        &session_b,
        Some("/recordings/generic_B".into()),
    )
    .await
    .expect("save session B");

    // The save must REFUSE (so the API layer creates a fresh meeting for session B).
    assert!(
        !attached_b,
        "a second session must not merge into the already-populated meeting"
    );

    // Symptom 1 (interwoven transcripts): meeting A keeps ONLY session A's segments.
    let transcript_a = TranscriptsRepository::get_full_transcript(pool, &meeting_a)
        .await
        .expect("full transcript A");
    assert!(
        transcript_a.contains("alice kicks off"),
        "A keeps its own transcript"
    );
    assert!(
        transcript_a.contains("bob shares"),
        "A keeps its own transcript"
    );
    assert!(
        !transcript_a.contains("totally unrelated"),
        "A must NOT absorb session B's transcript"
    );
    assert!(
        !transcript_a.contains("different people"),
        "A must NOT absorb session B's transcript"
    );

    let meta = MeetingsRepository::get_meeting_metadata(pool, &meeting_a)
        .await
        .expect("metadata A")
        .expect("meeting A exists");
    // Symptom 2 (lost/overwritten folder): folder_path not replaced by session B's.
    assert_eq!(
        meta.folder_path.as_deref(),
        Some("/recordings/weekly-sync_A"),
        "A's folder_path must not be overwritten by session B's folder"
    );
    // Symptom 3 (wrong attendees): the title and calendar link are preserved, so the
    // roster (seeded from calendar_event_id) cannot drift onto a generic meeting.
    assert_eq!(
        meta.title, "Weekly Sync",
        "A's title must not be overwritten"
    );
    assert_eq!(
        meta.calendar_event_id.as_deref(),
        Some("evt-AAA"),
        "A's calendar link must be preserved (no stale-event attendees)"
    );

    // The repo refusal itself creates no row; the API layer mints session B's row.
    let meetings = MeetingsRepository::get_meetings_enriched(pool)
        .await
        .expect("list");
    assert_eq!(
        meetings.len(),
        1,
        "repo refusal does not itself create a meeting"
    );
}

/// specs/0019 WS6.1 — the discard-safety decision over a real DB. A meeting may only be
/// auto-discarded when it is not calendar-linked, has no transcripts, and has no on-disk
/// audio; any of those present means "keep" (false). A missing row is safe (true).
#[tokio::test]
async fn recording_is_safe_to_discard_protects_durable_meetings() {
    let (_dir, db) = fresh_db().await;
    let pool = db.pool();

    // Missing row → nothing to protect → safe.
    assert!(
        recording_is_safe_to_discard(pool, "meeting-nope", None)
            .await
            .unwrap(),
        "a non-existent meeting is safe to discard"
    );

    // Plain empty ad-hoc meeting (no calendar link, no transcripts, no folder) → safe.
    let empty = MeetingsRepository::create_meeting(pool, None, None, None, None, None)
        .await
        .unwrap();
    assert!(
        recording_is_safe_to_discard(pool, &empty, None)
            .await
            .unwrap(),
        "an empty ad-hoc meeting is safe to discard"
    );

    // Calendar-linked meeting → keep, even with no transcripts.
    let cal = MeetingsRepository::create_meeting(
        pool,
        Some("Weekly Sync".into()),
        None,
        Some("recorded".into()),
        Some("evt-123".into()),
        None,
    )
    .await
    .unwrap();
    assert!(
        !recording_is_safe_to_discard(pool, &cal, None)
            .await
            .unwrap(),
        "a calendar-linked meeting must be kept"
    );

    // Meeting with persisted transcripts → keep.
    let withtx = MeetingsRepository::create_meeting(pool, None, None, None, None, None)
        .await
        .unwrap();
    TranscriptsRepository::save_transcripts_for_meeting(
        pool,
        &withtx,
        "Has Transcript",
        &[segment("hello there", 0.0, 1.0)],
        None,
    )
    .await
    .unwrap();
    assert!(
        !recording_is_safe_to_discard(pool, &withtx, None)
            .await
            .unwrap(),
        "a meeting with transcripts must be kept"
    );

    // Meeting with audio on disk (no transcripts) → keep. The passed-in folder wins.
    let audio_dir = tempfile::tempdir().unwrap();
    std::fs::write(audio_dir.path().join("audio.mp4"), b"not empty").unwrap();
    let withaudio = MeetingsRepository::create_meeting(pool, None, None, None, None, None)
        .await
        .unwrap();
    assert!(
        !recording_is_safe_to_discard(pool, &withaudio, audio_dir.path().to_str())
            .await
            .unwrap(),
        "a meeting whose folder holds audio must be kept"
    );
}

// specs/0024 WS6.1 (note 11) — summary auto-titling must rename an ad-hoc date-stamp meeting
// but never overwrite a title the user owns. The service guard (summary/service.rs) skips the
// rename when the meeting has a calendar_event_id OR `title_manually_set`. This locks the
// metadata those two conditions read.
#[tokio::test]
async fn manual_title_edit_is_protected_from_summary_rename() {
    let (_dir, db) = fresh_db().await;
    let pool = db.pool();

    // Ad-hoc meeting (no calendar link), default title → ELIGIBLE for auto-title.
    let adhoc = MeetingsRepository::create_meeting(pool, None, None, None, None, None)
        .await
        .expect("create ad-hoc meeting");
    let meta = MeetingsRepository::get_meeting_metadata(pool, &adhoc)
        .await
        .expect("metadata")
        .expect("row");
    assert!(meta.calendar_event_id.is_none());
    assert!(
        !meta.title_manually_set,
        "fresh ad-hoc meeting is not manually titled -> summary may auto-name it"
    );

    // User manually renames it → now PROTECTED.
    let renamed = MeetingsRepository::update_meeting_title(pool, &adhoc, "Quarterly planning")
        .await
        .expect("manual rename");
    assert!(renamed);
    let meta = MeetingsRepository::get_meeting_metadata(pool, &adhoc)
        .await
        .expect("metadata")
        .expect("row");
    assert_eq!(meta.title, "Quarterly planning");
    assert!(
        meta.title_manually_set,
        "a manual rename must set title_manually_set so summary won't overwrite it"
    );

    // The AI rename path (update_meeting_name) must NOT set the manual flag, so it can keep
    // refining an unowned title without ever locking itself out.
    let aiborn = MeetingsRepository::create_meeting(pool, None, None, None, None, None)
        .await
        .expect("create ai-titled meeting");
    MeetingsRepository::update_meeting_name(pool, &aiborn, "AI Generated Title")
        .await
        .expect("ai rename");
    let meta = MeetingsRepository::get_meeting_metadata(pool, &aiborn)
        .await
        .expect("metadata")
        .expect("row");
    assert!(
        !meta.title_manually_set,
        "the AI rename path must not mark the title as manually set"
    );
}

// specs/0024 WS2.1 (note 2) — a calendar-started recording must stay ONE object. The backend
// reuses an EMPTY, recent calendar-linked row instead of minting a duplicate, but must NEVER
// fold a row that already holds a recording (recurring events share one EventKit id).
#[tokio::test]
async fn dedupe_adopts_empty_calendar_row_but_not_a_recorded_one() {
    let (_dir, db) = fresh_db().await;
    let pool = db.pool();
    let event = "evt-recurring-1";

    // Pre-created (Join & Record) empty calendar row → adoptable.
    let pre = MeetingsRepository::create_meeting(
        pool,
        Some("Weekly sync".into()),
        None,
        None,
        Some(event.into()),
        None,
    )
    .await
    .expect("pre-create");
    assert_eq!(
        MeetingsRepository::find_adoptable_calendar_meeting(pool, event)
            .await
            .expect("lookup"),
        Some(pre.clone()),
        "an empty, recent calendar row must be adoptable"
    );

    // Once it holds transcripts it's a real recording → never adoptable/foldable.
    let attached = TranscriptsRepository::save_transcripts_for_meeting(
        pool,
        &pre,
        "Weekly sync",
        &[segment("hello team", 0.0, 1.0)],
        None,
    )
    .await
    .expect("save");
    assert!(attached);
    assert_eq!(
        MeetingsRepository::find_adoptable_calendar_meeting(pool, event)
            .await
            .expect("lookup"),
        None,
        "a populated recording of the same (recurring) event id must NOT be folded"
    );

    // A fresh empty row for the same event (e.g. next occurrence) is adoptable again — and the
    // lookup returns the empty one, not the populated prior recording.
    let next = MeetingsRepository::create_meeting(
        pool,
        Some("Weekly sync".into()),
        None,
        None,
        Some(event.into()),
        None,
    )
    .await
    .expect("next-create");
    assert_eq!(
        MeetingsRepository::find_adoptable_calendar_meeting(pool, event)
            .await
            .expect("lookup"),
        Some(next),
        "the empty row is adoptable; the recorded one is excluded"
    );
}

// specs/0032 (acceptance criterion 3) — the Join & Record identity invariants hold for
// Google-sourced events too: a `gcal:<calendarId>/<instanceId>` calendar_event_id is an
// opaque string to the meetings layer and must ride through create → adopt → record
// UNTOUCHED (adoption/dedupe/attendee routing all key off the exact stored value).
#[tokio::test]
async fn gcal_calendar_event_id_rides_through_adoption_untouched() {
    let (_dir, db) = fresh_db().await;
    let pool = db.pool();
    // A realistic Google id: calendar id (an email) + recurring-instance suffix.
    let event = "gcal:user@example.com/recur123_20260702T090000Z";

    // Join & Record pre-create: empty calendar-linked row.
    let pre = MeetingsRepository::create_meeting(
        pool,
        Some("Roadmap review".into()),
        None,
        None,
        Some(event.into()),
        None,
    )
    .await
    .expect("pre-create");

    // The stored id is byte-for-byte what was passed in.
    let meta = MeetingsRepository::get_meeting_metadata(pool, &pre)
        .await
        .expect("metadata")
        .expect("row");
    assert_eq!(
        meta.calendar_event_id.as_deref(),
        Some(event),
        "a gcal: id must be persisted untouched (routing depends on the exact string)"
    );

    // The 0024 WS2.1 adoption dedupe treats the id as opaque: the empty row is
    // adoptable for the same gcal: event id.
    assert_eq!(
        MeetingsRepository::find_adoptable_calendar_meeting(pool, event)
            .await
            .expect("lookup"),
        Some(pre.clone()),
        "an empty gcal:-linked row must be adoptable, exactly like an EventKit one"
    );

    // Once recorded it is ONE meeting: transcripts attach to the same row, the id
    // survives, and the recorded row is never adoptable/foldable again.
    let attached = TranscriptsRepository::save_transcripts_for_meeting(
        pool,
        &pre,
        "Roadmap review",
        &[segment("kicking off the roadmap review", 0.0, 2.0)],
        None,
    )
    .await
    .expect("save");
    assert!(attached);
    let meta = MeetingsRepository::get_meeting_metadata(pool, &pre)
        .await
        .expect("metadata")
        .expect("row");
    assert_eq!(
        meta.calendar_event_id.as_deref(),
        Some(event),
        "recording must not rewrite the gcal: calendar_event_id"
    );
    assert_eq!(
        MeetingsRepository::find_adoptable_calendar_meeting(pool, event)
            .await
            .expect("lookup"),
        None,
        "a populated gcal:-linked recording must not be folded"
    );
}

/// specs/0029 WS3.4 — the capture-channel tag round-trips through the repository:
/// insert (via the same `save_transcripts_for_meeting` the recording stop path uses,
/// against the REAL migrated schema) and read back, with the defensive allowlist
/// nulling unknown values and legacy/None staying NULL.
#[tokio::test]
async fn transcript_channel_round_trips_on_insert_and_read() {
    let (_dir, db) = fresh_db().await;
    let pool = db.pool();

    let meeting_id = MeetingsRepository::create_meeting(pool, None, None, None, None, None)
        .await
        .expect("create_meeting");

    let with_channel = |text: &str, start: f64, channel: Option<&str>| {
        let mut s = segment(text, start, start + 1.0);
        s.channel = channel.map(str::to_string);
        s
    };
    let segments = vec![
        with_channel("spoken by me", 0.0, Some("microphone")),
        with_channel("spoken remotely", 1.0, Some("system")),
        with_channel("overlapped speech", 2.0, Some("mixed")),
        with_channel("legacy/untagged", 3.0, None),
        with_channel("bogus tag", 4.0, Some("output")), // allowlist → NULL
    ];
    let attached = TranscriptsRepository::save_transcripts_for_meeting(
        pool,
        &meeting_id,
        "Channel round-trip",
        &segments,
        None,
    )
    .await
    .expect("save_transcripts_for_meeting");
    assert!(attached);

    let rows = sqlx::query_as::<_, (String, Option<String>)>(
        "SELECT transcript, channel FROM transcripts WHERE meeting_id = ? ORDER BY audio_start_time",
    )
    .bind(&meeting_id)
    .fetch_all(pool)
    .await
    .expect("read back transcripts.channel");

    assert_eq!(
        rows,
        vec![
            ("spoken by me".to_string(), Some("microphone".to_string())),
            ("spoken remotely".to_string(), Some("system".to_string())),
            ("overlapped speech".to_string(), Some("mixed".to_string())),
            ("legacy/untagged".to_string(), None),
            ("bogus tag".to_string(), None),
        ]
    );
}

/// specs/0029 WS7.2: deferred (first-time) transcription of a record-only meeting
/// persists through `replace_meeting_transcripts`, whose delete-then-insert must
/// tolerate a meeting with ZERO existing transcript rows — and stay a true
/// replacement when rows do exist.
#[tokio::test]
async fn retranscription_persist_tolerates_transcript_empty_meeting() {
    use app_lib::audio::retranscription::replace_meeting_transcripts;

    let (_dir, db) = fresh_db().await;
    let pool = db.pool();

    // A record-only meeting: created at recording start, never live-transcribed.
    let meeting_id = MeetingsRepository::create_meeting(pool, None, None, None, None, None)
        .await
        .expect("create_meeting");

    // `replace_meeting_transcripts` inserts ids verbatim (the retranscription path
    // generates them via create_transcript_segments), so give each row a unique id.
    let seg_with_id = |id: &str, text: &str, start: f64| {
        let mut s = segment(text, start, start + 1.0);
        s.id = id.to_string();
        s
    };

    // First-time transcription: DELETE on zero rows must be a clean no-op.
    let first = vec![
        seg_with_id("transcript-a", "hello from deferred transcription", 0.0),
        seg_with_id("transcript-b", "second segment", 2.0),
    ];
    replace_meeting_transcripts(pool, &meeting_id, &first)
        .await
        .expect("first-time persist on a transcript-empty meeting");

    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM transcripts WHERE meeting_id = ?")
        .bind(&meeting_id)
        .fetch_one(pool)
        .await
        .expect("count after first persist");
    assert_eq!(count, 2, "both segments persisted for the empty meeting");

    let full = TranscriptsRepository::get_full_transcript(pool, &meeting_id)
        .await
        .expect("full transcript");
    assert!(full.contains("hello from deferred transcription"));

    // A later re-run REPLACES (doesn't append to) the existing rows.
    let rerun = vec![seg_with_id("transcript-c", "replacement pass", 0.0)];
    replace_meeting_transcripts(pool, &meeting_id, &rerun)
        .await
        .expect("re-run persist over existing rows");

    let rows: Vec<(String,)> =
        sqlx::query_as("SELECT transcript FROM transcripts WHERE meeting_id = ?")
            .bind(&meeting_id)
            .fetch_all(pool)
            .await
            .expect("read back after re-run");
    assert_eq!(rows, vec![("replacement pass".to_string(),)]);
}

// The recording stop path hands `save_transcripts_for_meeting` the SESSION name, which used
// to overwrite the row's title unconditionally — turning a calendar-linked meeting ("Weekly
// sync" + attendees) into a date-stamped one while `calendar_event_id` survived. Titles the
// row already owns authoritatively (calendar-linked, or manually renamed) must survive the
// stop-save; unowned titles keep refreshing exactly as before.
#[tokio::test]
async fn stop_save_preserves_authoritative_titles() {
    let (_dir, db) = fresh_db().await;
    let pool = db.pool();
    let session_name = "Meeting 2026-07-01_22-05-10";

    // 1. Calendar-linked row (Join & Record pre-create): event title survives the save,
    //    while folder_path/updated_at still refresh.
    let cal = MeetingsRepository::create_meeting(
        pool,
        Some("Weekly sync".into()),
        None,
        Some("recorded".into()),
        Some("evt-title-guard".into()),
        None,
    )
    .await
    .expect("create calendar-linked meeting");
    let attached = TranscriptsRepository::save_transcripts_for_meeting(
        pool,
        &cal,
        session_name,
        &[segment("hello", 0.0, 1.0)],
        Some("/recordings/weekly-sync".into()),
    )
    .await
    .expect("save to calendar-linked meeting");
    assert!(attached);
    let meta = MeetingsRepository::get_meeting_metadata(pool, &cal)
        .await
        .expect("metadata")
        .expect("row");
    assert_eq!(
        meta.title, "Weekly sync",
        "the stop-save must not overwrite a calendar event title with the session name"
    );
    assert_eq!(
        meta.folder_path.as_deref(),
        Some("/recordings/weekly-sync"),
        "folder_path must still be recorded for a title-protected row"
    );

    // 2. Manually renamed row: the user's title survives the save.
    let renamed = MeetingsRepository::create_meeting(pool, None, None, None, None, None)
        .await
        .expect("create ad-hoc meeting");
    assert!(
        MeetingsRepository::update_meeting_title(pool, &renamed, "Budget review")
            .await
            .expect("manual rename")
    );
    let attached = TranscriptsRepository::save_transcripts_for_meeting(
        pool,
        &renamed,
        session_name,
        &[segment("numbers", 0.0, 1.0)],
        None,
    )
    .await
    .expect("save to renamed meeting");
    assert!(attached);
    let meta = MeetingsRepository::get_meeting_metadata(pool, &renamed)
        .await
        .expect("metadata")
        .expect("row");
    assert_eq!(
        meta.title, "Budget review",
        "the stop-save must not overwrite a manually-set title"
    );

    // 3. Plain ad-hoc row: no authoritative title -> the session name still refreshes it.
    let plain = MeetingsRepository::create_meeting(pool, None, None, None, None, None)
        .await
        .expect("create plain meeting");
    let attached = TranscriptsRepository::save_transcripts_for_meeting(
        pool,
        &plain,
        session_name,
        &[segment("ad-hoc", 0.0, 1.0)],
        None,
    )
    .await
    .expect("save to plain meeting");
    assert!(attached);
    let meta = MeetingsRepository::get_meeting_metadata(pool, &plain)
        .await
        .expect("metadata")
        .expect("row");
    assert_eq!(
        meta.title, session_name,
        "an unowned title keeps refreshing from the session name"
    );
}

/// specs/0046 WS2 Task 6 — `word_timestamps` round-trips through the transcript
/// repository: a segment saved with per-word JSON reads back byte-identical from
/// the DB column the diarization split (Task 7) will read. Segments without
/// words (Whisper / legacy) persist as NULL, unaffected.
#[tokio::test]
async fn word_timestamps_round_trip_through_save_and_load() {
    let (_dir, db) = fresh_db().await;
    let pool = db.pool();

    let meeting_id = MeetingsRepository::create_meeting(pool, None, None, None, None, None)
        .await
        .expect("create_meeting");

    let words_json = r#"[{"w":"hello","s":0.0,"e":0.4},{"w":"world","s":0.5,"e":0.9}]"#;
    let mut timed = segment("hello world", 0.0, 1.0);
    timed.word_timestamps = Some(words_json.to_string());
    let untimed = segment("no word stamps here", 1.0, 2.0);

    let attached = TranscriptsRepository::save_transcripts_for_meeting(
        pool,
        &meeting_id,
        "Word Timestamps",
        &[timed, untimed],
        None,
    )
    .await
    .expect("save_transcripts_for_meeting");
    assert!(attached);

    let loaded_timed: Option<String> = sqlx::query_scalar(
        "SELECT word_timestamps FROM transcripts WHERE meeting_id = ? AND transcript = ?",
    )
    .bind(&meeting_id)
    .bind("hello world")
    .fetch_one(pool)
    .await
    .expect("load word_timestamps for timed segment");
    assert_eq!(
        loaded_timed.as_deref(),
        Some(words_json),
        "word_timestamps should round-trip byte-identical"
    );

    let loaded_untimed: Option<String> = sqlx::query_scalar(
        "SELECT word_timestamps FROM transcripts WHERE meeting_id = ? AND transcript = ?",
    )
    .bind(&meeting_id)
    .bind("no word stamps here")
    .fetch_one(pool)
    .await
    .expect("load word_timestamps for untimed segment");
    assert_eq!(
        loaded_untimed, None,
        "a segment with no words should persist word_timestamps as NULL"
    );
}
