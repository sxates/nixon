//! Cached pre-call-prep briefs (specs/0036) — the `meeting_briefs` table.
//!
//! One row per target meeting (the upcoming/scheduled occurrence the brief is FOR). The
//! background generator (`aggregation::prep_jobs`) writes `ready`/`failed`/`none` rows keyed
//! by a `source_fingerprint` so it only regenerates when the inputs change. The prep IPC
//! (`aggregation::prep_commands`) reads them for the Prep tab.

use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::{Error as SqlxError, FromRow, SqlitePool};

/// A cached prep brief row.
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeetingBrief {
    pub meeting_id: String,
    /// 'pending' | 'ready' | 'failed' | 'none' (see the migration).
    pub status: String,
    pub brief_markdown: Option<String>,
    /// JSON array of `SourceMeeting` — the prior occurrences the brief drew from.
    pub sources_json: Option<String>,
    pub source_fingerprint: Option<String>,
    pub model_provider: Option<String>,
    pub model_name: Option<String>,
    pub generated_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    /// Consecutive failed generation passes (specs/0052). The background pass stops
    /// retrying at 3; reset to 0 on success or when `source_fingerprint` changes.
    pub failure_count: i64,
}

pub struct MeetingBriefsRepository;

impl MeetingBriefsRepository {
    /// The cached brief for a target meeting, if any.
    pub async fn get(
        pool: &SqlitePool,
        meeting_id: &str,
    ) -> Result<Option<MeetingBrief>, SqlxError> {
        sqlx::query_as::<_, MeetingBrief>(
            "SELECT meeting_id, status, brief_markdown, sources_json, source_fingerprint, \
                    model_provider, model_name, generated_at, created_at, updated_at, \
                    failure_count \
             FROM meeting_briefs WHERE meeting_id = ?",
        )
        .bind(meeting_id)
        .fetch_optional(pool)
        .await
    }

    /// Upsert a `ready` brief (markdown + sources + the input fingerprint + the model used).
    pub async fn upsert_ready(
        pool: &SqlitePool,
        meeting_id: &str,
        brief_markdown: &str,
        sources_json: &str,
        source_fingerprint: &str,
        model_provider: &str,
        model_name: &str,
    ) -> Result<(), SqlxError> {
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO meeting_briefs \
               (meeting_id, status, brief_markdown, sources_json, source_fingerprint, \
                model_provider, model_name, generated_at, created_at, updated_at) \
             VALUES (?, 'ready', ?, ?, ?, ?, ?, ?, ?, ?) \
             ON CONFLICT(meeting_id) DO UPDATE SET \
               status = 'ready', brief_markdown = excluded.brief_markdown, \
               sources_json = excluded.sources_json, source_fingerprint = excluded.source_fingerprint, \
               model_provider = excluded.model_provider, model_name = excluded.model_name, \
               generated_at = excluded.generated_at, updated_at = excluded.updated_at, \
               failure_count = 0",
        )
        .bind(meeting_id)
        .bind(brief_markdown)
        .bind(sources_json)
        .bind(source_fingerprint)
        .bind(model_provider)
        .bind(model_name)
        .bind(&now)
        .bind(&now)
        .bind(&now)
        .execute(pool)
        .await?;
        Ok(())
    }

    /// Increment the consecutive-failure counter and return its new value (specs/0052).
    /// Assumes the row exists — callers write `status = 'failed'` immediately before.
    pub async fn record_failure(pool: &SqlitePool, meeting_id: &str) -> Result<i64, SqlxError> {
        let count: i64 = sqlx::query_scalar(
            "UPDATE meeting_briefs SET failure_count = failure_count + 1, updated_at = ? \
             WHERE meeting_id = ? RETURNING failure_count",
        )
        .bind(Utc::now().to_rfc3339())
        .bind(meeting_id)
        .fetch_one(pool)
        .await?;
        Ok(count)
    }

    /// Clear the consecutive-failure counter — on success, on a fingerprint change, or on
    /// an explicit user retry (specs/0052).
    pub async fn reset_failures(pool: &SqlitePool, meeting_id: &str) -> Result<(), SqlxError> {
        sqlx::query(
            "UPDATE meeting_briefs SET failure_count = 0, updated_at = ? WHERE meeting_id = ?",
        )
        .bind(Utc::now().to_rfc3339())
        .bind(meeting_id)
        .execute(pool)
        .await?;
        Ok(())
    }

    /// Upsert a status-only row ('pending' | 'failed' | 'none'), carrying the input
    /// fingerprint so a 'none'/'failed' result isn't recomputed until the inputs change.
    /// Preserves any existing brief markdown (a transient 'failed' keeps the last good brief
    /// visible).
    pub async fn upsert_status(
        pool: &SqlitePool,
        meeting_id: &str,
        status: &str,
        source_fingerprint: Option<&str>,
    ) -> Result<(), SqlxError> {
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO meeting_briefs (meeting_id, status, source_fingerprint, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?) \
             ON CONFLICT(meeting_id) DO UPDATE SET \
               status = excluded.status, source_fingerprint = excluded.source_fingerprint, \
               updated_at = excluded.updated_at",
        )
        .bind(meeting_id)
        .bind(status)
        .bind(source_fingerprint)
        .bind(&now)
        .bind(&now)
        .execute(pool)
        .await?;
        Ok(())
    }

    /// Drop every cached brief whose target meeting belongs to `series_key` — by calendar
    /// stamp OR manual link (specs/0041 WS4 "regenerate on link"). Linking/unlinking changes
    /// the prior set for ALL of the series' occurrences, and `api_get_prep` only kicks off
    /// generation when NO brief row exists (a stale 'none'/'ready' row would otherwise sit
    /// until the next background pass), so the link commands delete the rows outright: the
    /// next open regenerates. Returns how many briefs were invalidated.
    pub async fn delete_for_series(pool: &SqlitePool, series_key: &str) -> Result<u64, SqlxError> {
        let result = sqlx::query(
            "DELETE FROM meeting_briefs WHERE meeting_id IN ( \
                 SELECT m.id FROM meetings m \
                 LEFT JOIN meeting_series_links l ON l.meeting_id = m.id \
                 WHERE m.calendar_series_key = ?1 OR l.series_key = ?1)",
        )
        .bind(series_key)
        .execute(pool)
        .await?;
        Ok(result.rows_affected())
    }
}

#[cfg(test)]
mod failure_count_tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    /// In-memory pool with migrations applied, and one meeting row to satisfy the FK.
    async fn pool_with_meeting(meeting_id: &str) -> SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory sqlite");
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .expect("migrations");
        let now = Utc::now().to_rfc3339();
        sqlx::query("INSERT INTO meetings (id, title, created_at, updated_at) VALUES (?, ?, ?, ?)")
            .bind(meeting_id)
            .bind("Weekly 1:1")
            .bind(&now)
            .bind(&now)
            .execute(&pool)
            .await
            .expect("seed meeting");
        pool
    }

    #[tokio::test]
    async fn record_failure_increments_and_reset_zeroes() {
        let pool = pool_with_meeting("m1").await;
        MeetingBriefsRepository::upsert_status(&pool, "m1", "failed", None)
            .await
            .unwrap();

        assert_eq!(
            MeetingBriefsRepository::record_failure(&pool, "m1")
                .await
                .unwrap(),
            1
        );
        assert_eq!(
            MeetingBriefsRepository::record_failure(&pool, "m1")
                .await
                .unwrap(),
            2
        );

        let brief = MeetingBriefsRepository::get(&pool, "m1")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(brief.failure_count, 2);

        MeetingBriefsRepository::reset_failures(&pool, "m1")
            .await
            .unwrap();
        let brief = MeetingBriefsRepository::get(&pool, "m1")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(brief.failure_count, 0);
    }

    #[tokio::test]
    async fn upsert_ready_resets_failure_count() {
        let pool = pool_with_meeting("m2").await;
        MeetingBriefsRepository::upsert_status(&pool, "m2", "failed", None)
            .await
            .unwrap();
        MeetingBriefsRepository::record_failure(&pool, "m2")
            .await
            .unwrap();

        MeetingBriefsRepository::upsert_ready(
            &pool,
            "m2",
            "# brief",
            "[]",
            "fp-1",
            "ollama",
            "gemma4:26b",
        )
        .await
        .unwrap();

        let brief = MeetingBriefsRepository::get(&pool, "m2")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            brief.failure_count, 0,
            "a successful generation clears the counter"
        );
    }
}
