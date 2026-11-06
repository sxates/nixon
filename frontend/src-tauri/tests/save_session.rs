// specs/0083 W2a task 4: `transcripts::save_session`, the body of `api_save_transcript`
// extracted so the Rust wrap-up can save without the webview. These pin the 0019 WS6.7
// guard, the 0037 resume append, and the create-new fall-through.
mod common;

use app_lib::database::repositories::meeting::MeetingsRepository;
use app_lib::transcripts::{save_session, SaveKind};
use common::{fresh_db, segment};

async fn rows(pool: &sqlx::SqlitePool, meeting_id: &str) -> Vec<(String, f64)> {
    sqlx::query_as(
        "SELECT transcript, audio_start_time FROM transcripts WHERE meeting_id = ? \
         ORDER BY audio_start_time",
    )
    .bind(meeting_id)
    .fetch_all(pool)
    .await
    .unwrap()
}

async fn title(pool: &sqlx::SqlitePool, meeting_id: &str) -> String {
    let (t,): (String,) = sqlx::query_as("SELECT title FROM meetings WHERE id = ?")
        .bind(meeting_id)
        .fetch_one(pool)
        .await
        .unwrap();
    t
}

async fn meeting_count(pool: &sqlx::SqlitePool) -> i64 {
    let (n,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM meetings")
        .fetch_one(pool)
        .await
        .unwrap();
    n
}

#[tokio::test]
async fn no_meeting_id_creates_a_new_meeting() {
    let (_d, db) = fresh_db().await;
    let pool = db.pool();
    let saved = save_session(
        pool,
        "Standup",
        &[segment("hello", 0.0, 1.0)],
        None,
        None,
        false,
        0.0,
    )
    .await
    .unwrap();
    assert_eq!(saved.kind, SaveKind::Created);
    assert_eq!(title(pool, &saved.meeting_id).await, "Standup");
    assert_eq!(rows(pool, &saved.meeting_id).await.len(), 1);

    // A blank id is treated as absent.
    let blank = save_session(
        pool,
        "B",
        &[segment("x", 0.0, 1.0)],
        None,
        Some("  "),
        false,
        0.0,
    )
    .await
    .unwrap();
    assert_eq!(blank.kind, SaveKind::Created);
    assert_eq!(meeting_count(pool).await, 2);
}

#[tokio::test]
async fn attaches_to_the_meeting_created_at_start() {
    let (_d, db) = fresh_db().await;
    let pool = db.pool();
    let id = MeetingsRepository::create_meeting(pool, None, None, None, None, None)
        .await
        .unwrap();
    let saved = save_session(
        pool,
        "Planning",
        &[segment("one", 0.0, 1.0), segment("two", 1.0, 2.0)],
        None,
        Some(&id),
        false,
        0.0,
    )
    .await
    .unwrap();
    assert_eq!(saved.kind, SaveKind::Attached);
    assert_eq!(saved.meeting_id, id);
    assert_eq!(title(pool, &id).await, "Planning");
    assert_eq!(rows(pool, &id).await.len(), 2);
    assert_eq!(meeting_count(pool).await, 1);
}

/// 0019 WS6.7: a second, non-resumed save to an already-populated meeting never merges;
/// it lands in a fresh row and the first session is untouched. Re-running a save is thus
/// never a duplicate on the original meeting.
#[tokio::test]
async fn populated_meeting_is_never_merged_into() {
    let (_d, db) = fresh_db().await;
    let pool = db.pool();
    let id = MeetingsRepository::create_meeting(pool, None, None, None, None, None)
        .await
        .unwrap();
    let segs = [segment("first session", 0.0, 1.0)];
    save_session(pool, "A", &segs, None, Some(&id), false, 0.0)
        .await
        .unwrap();
    let again = save_session(pool, "A", &segs, None, Some(&id), false, 0.0)
        .await
        .unwrap();
    assert_eq!(again.kind, SaveKind::Created);
    assert_ne!(again.meeting_id, id);
    assert_eq!(rows(pool, &id).await.len(), 1, "original meeting unchanged");
}

/// 0037: a resumed session appends to the same meeting, shifted by the offset.
#[tokio::test]
async fn resumed_session_appends_with_the_offset() {
    let (_d, db) = fresh_db().await;
    let pool = db.pool();
    let id = MeetingsRepository::create_meeting(pool, None, None, None, None, None)
        .await
        .unwrap();
    save_session(
        pool,
        "A",
        &[segment("before", 0.0, 1.0)],
        None,
        Some(&id),
        false,
        0.0,
    )
    .await
    .unwrap();
    let saved = save_session(
        pool,
        "A",
        &[segment("after", 0.0, 1.0)],
        None,
        Some(&id),
        true,
        30.0,
    )
    .await
    .unwrap();
    assert_eq!(saved.kind, SaveKind::Appended);
    assert_eq!(saved.meeting_id, id);
    let r = rows(pool, &id).await;
    assert_eq!(r.len(), 2);
    assert_eq!(r[1].0, "after");
    assert!((r[1].1 - 30.0).abs() < 1e-9, "offset applied: {}", r[1].1);
}

/// 0037: a resume whose meeting is gone falls through to create-new, so nothing is lost.
#[tokio::test]
async fn resume_of_a_missing_meeting_creates_a_new_one() {
    let (_d, db) = fresh_db().await;
    let pool = db.pool();
    let saved = save_session(
        pool,
        "Lost",
        &[segment("kept", 0.0, 1.0)],
        None,
        Some("meeting-gone"),
        true,
        10.0,
    )
    .await
    .unwrap();
    assert_eq!(saved.kind, SaveKind::Created);
    assert_ne!(saved.meeting_id, "meeting-gone");
    assert_eq!(rows(pool, &saved.meeting_id).await.len(), 1);
}
