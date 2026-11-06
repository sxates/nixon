//! The wrap-up stage sequence against a real (temp) database, with the stop body faked:
//! no webview, no audio hardware (specs/0083 §Verification).

use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use sqlx::SqlitePool;
use tauri::test::{mock_app, MockRuntime};
use tauri::{Listener, Manager};

use super::run::{run, Inputs};
use super::stop_action::HANDOFF_FAILED_TOAST;
use super::test_support::{reset_slot, SERIAL};
use super::*;
use crate::database::manager::DatabaseManager;
use crate::state::AppState;
use crate::transcripts::TranscriptSegment;

struct Fixture {
    dir: tempfile::TempDir,
    app: tauri::App<MockRuntime>,
    pool: SqlitePool,
    events: Arc<StdMutex<Vec<serde_json::Value>>>,
}

async fn fixture() -> Fixture {
    reset_slot();
    let dir = tempfile::tempdir().unwrap();
    let db = DatabaseManager::new(
        dir.path().join("t.sqlite").to_str().unwrap(),
        dir.path().join("none.db").to_str().unwrap(),
    )
    .await
    .unwrap();
    let pool = db.pool().clone();
    let app = mock_app();
    app.manage(AppState { db_manager: db });
    let events = Arc::new(StdMutex::new(Vec::new()));
    let sink = events.clone();
    app.handle().listen_any(EVENT_WRAP_UP, move |e| {
        sink.lock()
            .unwrap()
            .push(serde_json::from_str(e.payload()).unwrap());
    });
    Fixture {
        dir,
        app,
        pool,
        events,
    }
}

impl Fixture {
    fn stages(&self) -> Vec<String> {
        self.events
            .lock()
            .unwrap()
            .iter()
            .map(|e| e["stage"].as_str().unwrap().to_string())
            .collect()
    }

    fn event(&self, stage: &str) -> serde_json::Value {
        self.events
            .lock()
            .unwrap()
            .iter()
            .find(|e| e["stage"] == stage)
            .cloned()
            .unwrap_or_else(|| panic!("no {stage} event in {:?}", self.stages()))
    }

    async fn wrap_up(&self, stopped: Result<StoppedSession, String>, inputs: Inputs) {
        let state = match begin(StopSource::User, None) {
            Begin::Started(s) => s,
            Begin::InFlight(_) => panic!("a wrap-up is already in flight"),
        };
        run(
            Reporter::new(self.app.handle().clone(), state),
            inputs,
            async move { stopped },
        )
        .await;
    }
}

fn inputs(deferral: (bool, bool)) -> Inputs {
    Inputs {
        snapshot: SessionSnapshot::default(),
        deferral,
        handoff_wait: Duration::from_millis(50),
    }
}

/// The meeting row `start_recording` creates, with no transcripts yet.
async fn meeting(pool: &SqlitePool, id: &str, mode: Option<&str>) {
    sqlx::query(
        "INSERT INTO meetings (id, title, created_at, updated_at, processing_mode) \
         VALUES (?, 'Placeholder', '2026-11-05T10:00:00Z', '2026-11-05T10:00:00Z', ?)",
    )
    .bind(id)
    .bind(mode)
    .execute(pool)
    .await
    .unwrap();
}

fn seg(text: &str, start: f64) -> TranscriptSegment {
    TranscriptSegment {
        id: format!("seg_{start}"),
        text: text.into(),
        timestamp: "[00:00]".into(),
        audio_start_time: Some(start),
        audio_end_time: Some(start + 1.0),
        duration: Some(1.0),
        ..Default::default()
    }
}

fn stopped(meeting_id: &str, texts: &[&str]) -> StoppedSession {
    StoppedSession {
        meeting_id: Some(meeting_id.into()),
        meeting_name: Some("Weekly sync".into()),
        segments: texts
            .iter()
            .enumerate()
            .map(|(i, t)| seg(t, i as f64))
            .collect(),
        drain: Some(DrainOutcome::Completed),
        ..Default::default()
    }
}

async fn rows(pool: &SqlitePool, id: &str) -> Vec<(String, f64)> {
    sqlx::query_as(
        "SELECT transcript, audio_start_time FROM transcripts WHERE meeting_id = ? \
         ORDER BY audio_start_time",
    )
    .bind(id)
    .fetch_all(pool)
    .await
    .unwrap()
}

async fn mode(pool: &SqlitePool, id: &str) -> Option<String> {
    sqlx::query_scalar("SELECT processing_mode FROM meetings WHERE id = ?")
        .bind(id)
        .fetch_one(pool)
        .await
        .unwrap()
}

#[tokio::test]
async fn a_normal_stop_saves_diarizes_and_gates_the_summary_without_a_webview() {
    let _serial = SERIAL.lock().await;
    let f = fixture().await;
    meeting(&f.pool, "m1", None).await;

    f.wrap_up(
        Ok(stopped("m1", &["hello", "world"])),
        inputs((false, false)),
    )
    .await;

    assert_eq!(
        f.stages(),
        ["saving", "saved", "diarizing", "summary-skipped", "done"]
    );
    assert_eq!(rows(&f.pool, "m1").await.len(), 2);
    let saved = f.event("saved");
    assert_eq!(saved["meetingId"], "m1");
    assert_eq!(saved["title"], "Weekly sync");
    assert_eq!(saved["toast"], serde_json::Value::Null);
    // No model is configured in a fresh DB.
    let skipped = f.event("summary-skipped");
    assert_eq!(skipped["reason"], "no-model");
    assert!(skipped["toast"]["title"]
        .as_str()
        .unwrap()
        .contains("no AI model"));
    assert_eq!(
        f.event("done")["toast"],
        serde_json::Value::Null,
        "toasts are per-stage"
    );
    assert_eq!(mode(&f.pool, "m1").await, None);
}

#[tokio::test]
async fn a_timed_out_drain_still_saves_and_says_so() {
    let _serial = SERIAL.lock().await;
    let f = fixture().await;
    meeting(&f.pool, "m1", None).await;
    let mut s = stopped("m1", &["partial"]);
    s.drain = Some(DrainOutcome::TimedOut);

    f.wrap_up(Ok(s), inputs((false, false))).await;

    let saved = f.event("saved");
    assert_eq!(saved["transcriptionIncomplete"], true);
    assert!(saved["toast"]["title"]
        .as_str()
        .unwrap()
        .contains("fully finish"));
    assert_eq!(rows(&f.pool, "m1").await.len(), 1);
}

#[tokio::test]
async fn a_failed_stop_still_saves_what_transcripts_json_holds() {
    let _serial = SERIAL.lock().await;
    let f = fixture().await;
    meeting(&f.pool, "m1", None).await;
    let folder = f.dir.path().join("rec");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(
        folder.join("transcripts.json"),
        serde_json::json!({ "segments": [
            { "id": "a", "text": "captured before the failure", "audio_start_time": 0.0,
              "audio_end_time": 1.0, "duration": 1.0, "display_time": "[00:00]" }
        ]})
        .to_string(),
    )
    .unwrap();
    let mut i = inputs((false, false));
    i.snapshot = SessionSnapshot {
        meeting_id: Some("m1".into()),
        folder_path: Some(folder.to_string_lossy().into_owned()),
        ..Default::default()
    };

    f.wrap_up(Err("Failed to stop audio streams".into()), i)
        .await;

    assert_eq!(
        rows(&f.pool, "m1").await[0].0,
        "captured before the failure"
    );
    assert!(f.stages().contains(&"saved".to_string()));
}

#[tokio::test]
async fn an_empty_session_with_nothing_to_keep_is_discarded() {
    let _serial = SERIAL.lock().await;
    let f = fixture().await;
    meeting(&f.pool, "m1", None).await;

    f.wrap_up(Ok(stopped("m1", &[])), inputs((false, false)))
        .await;

    assert_eq!(f.stages(), ["saving", "discarded", "done"]);
    let left: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM meetings WHERE id = 'm1'")
        .fetch_one(&f.pool)
        .await
        .unwrap();
    assert_eq!(left, 0);
}

#[tokio::test]
async fn an_empty_session_with_typed_notes_is_kept() {
    let _serial = SERIAL.lock().await;
    let f = fixture().await;
    meeting(&f.pool, "m1", None).await;
    crate::database::repositories::meeting_note::MeetingNotesRepository::upsert_notes(
        &f.pool,
        "m1",
        Some("my agenda"),
        None,
    )
    .await
    .unwrap();

    f.wrap_up(Ok(stopped("m1", &[])), inputs((false, false)))
        .await;

    assert!(
        f.stages().contains(&"saved".to_string()),
        "{:?}",
        f.stages()
    );
    let left: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM meetings WHERE id = 'm1'")
        .fetch_one(&f.pool)
        .await
        .unwrap();
    assert_eq!(left, 1);
}

#[tokio::test]
async fn a_resumed_session_appends_to_the_same_meeting_with_its_offset() {
    let _serial = SERIAL.lock().await;
    let f = fixture().await;
    meeting(&f.pool, "m1", None).await;
    crate::transcripts::save_session(
        &f.pool,
        "Weekly sync",
        &[seg("first session", 0.0)],
        None,
        Some("m1"),
        false,
        0.0,
    )
    .await
    .unwrap();
    let mut s = stopped("m1", &["after resume"]);
    s.resumed = true;
    s.prior_audio_duration_seconds = 60.0;

    f.wrap_up(Ok(s), inputs((false, false))).await;

    let r = rows(&f.pool, "m1").await;
    assert_eq!(r.len(), 2);
    assert_eq!(r[1], ("after resume".to_string(), 60.0));
    let meetings: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM meetings")
        .fetch_one(&f.pool)
        .await
        .unwrap();
    assert_eq!(meetings, 1, "never a duplicate meeting");
}

#[tokio::test]
async fn a_still_deferred_session_is_marked_and_nothing_runs() {
    let _serial = SERIAL.lock().await;
    let f = fixture().await;
    meeting(&f.pool, "m1", None).await;

    f.wrap_up(Ok(stopped("m1", &["x"])), inputs((true, true)))
        .await;

    assert_eq!(f.stages(), ["saving", "saved", "summary-skipped", "done"]);
    assert_eq!(mode(&f.pool, "m1").await.as_deref(), Some("defer"));
    let skipped = f.event("summary-skipped");
    assert_eq!(skipped["reason"], "deferred");
    assert_eq!(skipped["toast"]["title"], "Processing deferred");
}

#[tokio::test]
async fn an_unanswered_process_now_handoff_degrades_to_diarize_and_warns() {
    let _serial = SERIAL.lock().await;
    let f = fixture().await;
    meeting(&f.pool, "m1", Some("live")).await;

    f.wrap_up(Ok(stopped("m1", &["x"])), inputs((true, true)))
        .await;

    assert_eq!(
        f.stages(),
        [
            "saving",
            "saved",
            "handoff",
            "diarizing",
            "summary-skipped",
            "done"
        ]
    );
    assert_eq!(
        mode(&f.pool, "m1").await.as_deref(),
        Some("defer"),
        "0051 WS2"
    );
    assert_eq!(
        f.event("diarizing")["toast"]["title"],
        HANDOFF_FAILED_TOAST.title
    );
    assert_eq!(f.event("summary-skipped")["reason"], "deferred");
}

#[tokio::test]
async fn an_accepted_process_now_handoff_runs_nothing_more_and_stays_quiet() {
    let _serial = SERIAL.lock().await;
    let f = fixture().await;
    meeting(&f.pool, "m1", Some("live")).await;
    f.app.handle().listen_any(EVENT_WRAP_UP, |e| {
        let v: serde_json::Value = serde_json::from_str(e.payload()).unwrap();
        if v["stage"] == "handoff" {
            handoff::answer("m1", true);
        }
    });
    let mut i = inputs((true, true));
    i.handoff_wait = Duration::from_secs(5);

    f.wrap_up(Ok(stopped("m1", &["x"])), i).await;

    assert_eq!(
        f.stages(),
        ["saving", "saved", "handoff", "summary-skipped", "done"]
    );
    let skipped = f.event("summary-skipped");
    assert_eq!(skipped["reason"], "deferred-processing");
    assert_eq!(skipped["toast"], serde_json::Value::Null);
}

#[tokio::test]
async fn the_pinned_summary_language_is_written_to_the_folder() {
    let _serial = SERIAL.lock().await;
    let f = fixture().await;
    meeting(&f.pool, "m1", None).await;
    let folder = f.dir.path().join("rec");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("metadata.json"), "{}").unwrap();
    prefs::mirror(&crate::audio::recording_preferences::RecordingPreferences {
        summary_language_default: Some("de".into()),
        ..Default::default()
    });
    let mut s = stopped("m1", &["hello"]);
    s.folder_path = Some(folder.to_string_lossy().into_owned());

    f.wrap_up(Ok(s), inputs((false, false))).await;
    prefs::mirror(&Default::default());

    assert_eq!(
        crate::summary::metadata::read_summary_language_from_metadata(&folder).unwrap(),
        Some("de".into())
    );
}

#[tokio::test]
async fn capture_stopped_moves_a_stopping_wrap_up_to_draining_and_nothing_else() {
    let _serial = SERIAL.lock().await;
    let f = fixture().await;
    capture_stopped(f.app.handle()); // no wrap-up (a dev-fixture stop): no-op
    assert!(f.stages().is_empty());
    let _ = begin(StopSource::User, Some("m1".into()));
    capture_stopped(f.app.handle());
    assert_eq!(f.stages(), ["draining"]);
    assert_eq!(lock_slot().as_ref().map(|s| s.stage), Some(Stage::Draining));
    capture_stopped(f.app.handle());
    assert_eq!(f.stages(), ["draining"], "only from stopping");
    reset_slot();
}

#[tokio::test]
async fn a_stop_with_no_recording_and_no_wrap_up_is_refused() {
    let _serial = SERIAL.lock().await;
    let f = fixture().await;
    let err = request_stop(f.app.handle(), StopSource::Tray)
        .await
        .unwrap_err();
    assert_eq!(err, "No recording in progress");
    let first = match begin(StopSource::User, Some("m1".into())) {
        Begin::Started(s) => s,
        Begin::InFlight(_) => unreachable!(),
    };
    // The recording flag is already down while the stop body drains: join the wrap-up.
    let ticket = request_stop(f.app.handle(), StopSource::Tray)
        .await
        .unwrap();
    assert_eq!(ticket, WrapUpTicket::of(&first));
    reset_slot();
}

#[tokio::test]
async fn stop_requests_are_single_flight_and_block_a_new_start_until_saved() {
    let _serial = SERIAL.lock().await;
    reset_slot();
    let first = match begin(StopSource::Tray, Some("m1".into())) {
        Begin::Started(s) => s,
        Begin::InFlight(_) => panic!("nothing was in flight"),
    };
    assert!(refuse_start_while_saving().is_err());
    match begin(StopSource::MeetingEnded(Platform::Zoom), None) {
        Begin::InFlight(s) => assert_eq!(s.id, first.id),
        Begin::Started(_) => panic!("a second stop must join the first"),
    }
    let app = mock_app();
    let mut rep = Reporter::new(app.handle().clone(), first.clone());
    rep.stage(Stage::Saving, |_| {});
    assert!(refuse_start_while_saving().is_err());
    rep.stage(Stage::Saved, |_| {});
    assert!(
        refuse_start_while_saving().is_ok(),
        "diarizing/summarizing don't block"
    );
    assert_eq!(
        lock_slot().as_ref().map(|s| s.stage),
        Some(Stage::Saved),
        "hydration reads the current stage"
    );
    match begin(StopSource::User, None) {
        Begin::Started(s) => assert_ne!(s.id, first.id),
        Begin::InFlight(_) => panic!("a saved wrap-up no longer holds the slot"),
    }
    reset_slot();
}
