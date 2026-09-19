//! Persistence for the Auto summary outline (specs/0053 W3).

use anyhow::{Context, Result};
use sqlx::SqlitePool;

/// A previously derived outline.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredOutline {
    /// Serialized `crate::summary::outline::Outline`.
    pub outline_json: String,
    /// Denormalized from the outline so the action-item gate can read it
    /// without deserializing.
    pub has_commitments: bool,
    pub derived_at: String,
}

pub struct SummaryOutlineRepository;

impl SummaryOutlineRepository {
    pub async fn get(pool: &SqlitePool, meeting_id: &str) -> Result<Option<StoredOutline>> {
        let row: Option<(String, i64, String)> = sqlx::query_as(
            "SELECT outline_json, has_commitments, derived_at \
             FROM meeting_summary_outlines WHERE meeting_id = ?",
        )
        .bind(meeting_id)
        .fetch_optional(pool)
        .await
        .context("Failed to read the summary outline")?;

        Ok(row.map(
            |(outline_json, has_commitments, derived_at)| StoredOutline {
                outline_json,
                has_commitments: has_commitments != 0,
                derived_at,
            },
        ))
    }

    pub async fn upsert(
        pool: &SqlitePool,
        meeting_id: &str,
        outline_json: &str,
        has_commitments: bool,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO meeting_summary_outlines \
                 (meeting_id, outline_json, has_commitments, derived_at) \
             VALUES (?, ?, ?, datetime('now')) \
             ON CONFLICT(meeting_id) DO UPDATE SET \
                 outline_json = excluded.outline_json, \
                 has_commitments = excluded.has_commitments, \
                 derived_at = excluded.derived_at",
        )
        .bind(meeting_id)
        .bind(outline_json)
        .bind(i64::from(has_commitments))
        .execute(pool)
        .await
        .context("Failed to save the summary outline")?;
        Ok(())
    }

    pub async fn delete(pool: &SqlitePool, meeting_id: &str) -> Result<()> {
        sqlx::query("DELETE FROM meeting_summary_outlines WHERE meeting_id = ?")
            .bind(meeting_id)
            .execute(pool)
            .await
            .context("Failed to clear the summary outline")?;
        Ok(())
    }
}
