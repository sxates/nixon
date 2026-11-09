// Resolving a `nixon://meeting/<id>?t=<seconds>` link to the transcript segment to
// scroll to (specs/0087). Read-only: opening a link only navigates.

use serde::Serialize;
use sqlx::SqlitePool;

use crate::database::repositories::transcript::TranscriptsRepository;
use crate::state::AppState;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MeetingLinkTarget {
    /// The meeting row exists.
    pub exists: bool,
    /// Set only when a time was given and a timed segment resolves.
    pub segment_id: Option<String>,
}

pub async fn resolve_meeting_link(
    pool: &SqlitePool,
    meeting_id: &str,
    t: Option<f64>,
) -> Result<MeetingLinkTarget, sqlx::Error> {
    let exists = sqlx::query_scalar::<_, String>("SELECT id FROM meetings WHERE id = ?")
        .bind(meeting_id)
        .fetch_optional(pool)
        .await?
        .is_some();
    let segment_id = match t {
        Some(t) if exists && t.is_finite() => {
            TranscriptsRepository::segment_id_at_time(pool, meeting_id, t).await?
        }
        _ => None,
    };
    Ok(MeetingLinkTarget { exists, segment_id })
}

/// Backend half of opening a meeting link: does the meeting exist, and which segment
/// does `t` (seconds from recording start) fall in.
#[tauri::command]
pub async fn api_resolve_meeting_link(
    state: tauri::State<'_, AppState>,
    meeting_id: String,
    t: Option<f64>,
) -> Result<MeetingLinkTarget, String> {
    resolve_meeting_link(state.db_manager.pool(), &meeting_id, t)
        .await
        .map_err(|e| format!("Couldn't look up that meeting link: {}", e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::repositories::meeting::MeetingsRepository;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn test_pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("open in-memory sqlite");
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .expect("migrations");
        pool
    }

    async fn meeting(pool: &SqlitePool) -> String {
        MeetingsRepository::create_meeting(pool, Some("M".into()), None, None, None, None)
            .await
            .expect("create_meeting")
    }

    async fn seg(pool: &SqlitePool, meeting_id: &str, id: &str, start: Option<f64>) {
        sqlx::query(
            "INSERT INTO transcripts (id, meeting_id, transcript, timestamp, audio_start_time)
             VALUES (?, ?, 'x', '2026-01-01T00:00:00Z', ?)",
        )
        .bind(id)
        .bind(meeting_id)
        .bind(start)
        .execute(pool)
        .await
        .expect("insert segment");
    }

    async fn three(pool: &SqlitePool) -> String {
        let m = meeting(pool).await;
        seg(pool, &m, "a", Some(0.0)).await;
        seg(pool, &m, "b", Some(10.0)).await;
        seg(pool, &m, "c", Some(20.0)).await;
        m
    }

    #[tokio::test]
    async fn resolves_segment_containing_t() {
        let pool = test_pool().await;
        let m = three(&pool).await;
        let r = resolve_meeting_link(&pool, &m, Some(15.0)).await.unwrap();
        assert!(r.exists);
        assert_eq!(r.segment_id.as_deref(), Some("b"));
        let r = resolve_meeting_link(&pool, &m, Some(10.0)).await.unwrap();
        assert_eq!(r.segment_id.as_deref(), Some("b"));
    }

    #[tokio::test]
    async fn t_past_end_resolves_last_segment() {
        let pool = test_pool().await;
        let m = three(&pool).await;
        let r = resolve_meeting_link(&pool, &m, Some(999.0)).await.unwrap();
        assert_eq!(r.segment_id.as_deref(), Some("c"));
    }

    #[tokio::test]
    async fn t_before_first_resolves_first() {
        let pool = test_pool().await;
        let m = meeting(&pool).await;
        seg(&pool, &m, "b", Some(10.0)).await;
        seg(&pool, &m, "c", Some(20.0)).await;
        let r = resolve_meeting_link(&pool, &m, Some(2.0)).await.unwrap();
        assert_eq!(r.segment_id.as_deref(), Some("b"));
    }

    #[tokio::test]
    async fn no_timed_segments_is_none() {
        let pool = test_pool().await;
        let m = meeting(&pool).await;
        seg(&pool, &m, "a", None).await;
        let r = resolve_meeting_link(&pool, &m, Some(5.0)).await.unwrap();
        assert!(r.exists);
        assert_eq!(r.segment_id, None);
    }

    #[tokio::test]
    async fn unknown_meeting_is_not_exists() {
        let pool = test_pool().await;
        let r = resolve_meeting_link(&pool, "nope", Some(5.0))
            .await
            .unwrap();
        assert_eq!(
            r,
            MeetingLinkTarget {
                exists: false,
                segment_id: None
            }
        );
    }

    #[tokio::test]
    async fn meeting_without_transcript_exists_without_segment() {
        let pool = test_pool().await;
        let m = meeting(&pool).await;
        let r = resolve_meeting_link(&pool, &m, Some(5.0)).await.unwrap();
        assert!(r.exists);
        assert_eq!(r.segment_id, None);
    }

    #[tokio::test]
    async fn no_time_means_no_segment() {
        let pool = test_pool().await;
        let m = three(&pool).await;
        let r = resolve_meeting_link(&pool, &m, None).await.unwrap();
        assert!(r.exists);
        assert_eq!(r.segment_id, None);
    }
}
