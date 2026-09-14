//! 1.10 feedback — batch retranscription must persist the per-segment capture
//! channel so the offline diarization pass can attribute the local user's
//! segments to "You" (align.rs mic short-circuit). Before this, the transcript
//! rows a deferred (battery/record-only) meeting got from retranscription were
//! always `channel = NULL`, and every one of the user's own lines ended up
//! under "Unknown speaker".
//!
//! Run with:
//!   cargo test --features metal --test retranscription_persistence

mod common;

use app_lib::audio::retranscription::{
    clear_deferred_marker_after_transcription, replace_meeting_transcripts,
};
use app_lib::database::repositories::meeting::MeetingsRepository;
use common::{fresh_db, segment};

#[tokio::test]
async fn replace_meeting_transcripts_persists_channel_tags() {
    let (_dir, db) = fresh_db().await;
    let pool = db.pool();

    let meeting_id = MeetingsRepository::create_meeting(pool, None, None, None, None, None)
        .await
        .expect("create_meeting");

    // Segments as run_retranscription builds them post channel-tagging: ids set,
    // a mic-tagged line, a system-tagged line, and an untagged (silent-window) one.
    let mut segments = vec![
        segment("hello from me", 0.0, 2.0),
        segment("hello from the remote side", 2.0, 4.0),
        segment("mumble", 4.0, 5.0),
    ];
    for (i, seg) in segments.iter_mut().enumerate() {
        seg.id = format!("transcript-test-{i}");
    }
    segments[0].channel = Some("microphone".to_string());
    segments[1].channel = Some("system".to_string());
    segments[2].channel = None;

    replace_meeting_transcripts(pool, &meeting_id, &segments)
        .await
        .expect("replace_meeting_transcripts");

    let rows: Vec<(String, Option<String>)> = sqlx::query_as(
        "SELECT transcript, channel FROM transcripts WHERE meeting_id = ? ORDER BY audio_start_time",
    )
    .bind(&meeting_id)
    .fetch_all(pool)
    .await
    .expect("read back transcripts");

    assert_eq!(
        rows,
        vec![
            ("hello from me".to_string(), Some("microphone".to_string())),
            (
                "hello from the remote side".to_string(),
                Some("system".to_string())
            ),
            ("mumble".to_string(), None),
        ],
        "retranscribed rows must carry their capture-channel tags"
    );
}

// 1.10 feedback — "it should only happen once then be done": ANY successful
// retranscription (backlog, the meeting's "Process now"/"Transcribe now"
// buttons, or the summary flow's transcribe-first step) must clear the
// meeting's 'defer' marker, so returning to AC power can never re-discover —
// and re-process — a meeting whose transcript already exists.
#[tokio::test]
async fn successful_retranscription_clears_the_defer_marker() {
    let (_dir, db) = fresh_db().await;
    let pool = db.pool();

    let meeting_id = MeetingsRepository::create_meeting(pool, None, None, None, None, None)
        .await
        .expect("create_meeting");
    MeetingsRepository::set_processing_mode(pool, &meeting_id, Some("defer"))
        .await
        .expect("mark deferred");

    clear_deferred_marker_after_transcription(pool, &meeting_id).await;

    let mode = MeetingsRepository::get_processing_mode(pool, &meeting_id)
        .await
        .expect("read processing mode");
    assert_eq!(mode, None, "defer marker must be cleared after transcription");
}

#[tokio::test]
async fn marker_clear_is_a_noop_for_unmarked_meetings() {
    let (_dir, db) = fresh_db().await;
    let pool = db.pool();

    // "Enhance"/import retranscriptions run on meetings with no marker at all —
    // the clear must be harmless there.
    let meeting_id = MeetingsRepository::create_meeting(pool, None, None, None, None, None)
        .await
        .expect("create_meeting");

    clear_deferred_marker_after_transcription(pool, &meeting_id).await;

    let mode = MeetingsRepository::get_processing_mode(pool, &meeting_id)
        .await
        .expect("read processing mode");
    assert_eq!(mode, None);
}
