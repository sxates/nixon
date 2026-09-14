use crate::database::models::MeetingNote;
use chrono::Utc;
use sqlx::SqlitePool;
use tracing::info as log_info;

pub struct MeetingNotesRepository;

impl MeetingNotesRepository {
    /// Upserts the user's raw notes for a meeting (autosaved from the live notepad).
    /// Preserves the enhanced_* columns. Returns false if the meeting does not exist.
    pub async fn upsert_notes(
        pool: &SqlitePool,
        meeting_id: &str,
        notes_markdown: Option<&str>,
        notes_json: Option<&str>,
    ) -> Result<bool, sqlx::Error> {
        let meeting_exists: bool = sqlx::query("SELECT 1 FROM meetings WHERE id = ?")
            .bind(meeting_id)
            .fetch_optional(pool)
            .await?
            .is_some();

        if !meeting_exists {
            log_info!(
                "upsert_notes: meeting_id {} does not exist; skipping",
                meeting_id
            );
            return Ok(false);
        }

        let now = Utc::now().to_rfc3339();
        sqlx::query(
            r#"
            INSERT INTO meeting_notes (meeting_id, notes_markdown, notes_json, created_at, updated_at)
            VALUES (?, ?, ?, ?, ?)
            ON CONFLICT(meeting_id) DO UPDATE SET
                notes_markdown = excluded.notes_markdown,
                notes_json = excluded.notes_json,
                updated_at = excluded.updated_at
            "#,
        )
        .bind(meeting_id)
        .bind(notes_markdown)
        .bind(notes_json)
        .bind(&now)
        .bind(&now)
        .execute(pool)
        .await?;

        Ok(true)
    }

    /// Upserts the meeting's PREP notes (specs/0036) — the pre-call agenda, autosaved from the
    /// Prep editor. Writes only the `prep_*` columns, leaving live `notes_markdown` untouched
    /// (and vice-versa: [`upsert_notes`] never touches prep). Returns false if the meeting does
    /// not exist. On a fresh row `notes_markdown` stays NULL; on conflict only the prep columns
    /// change (the row's `updated_at`, the live-notes timestamp, is preserved).
    pub async fn upsert_prep_notes(
        pool: &SqlitePool,
        meeting_id: &str,
        prep_markdown: Option<&str>,
        prep_json: Option<&str>,
    ) -> Result<bool, sqlx::Error> {
        let meeting_exists: bool = sqlx::query("SELECT 1 FROM meetings WHERE id = ?")
            .bind(meeting_id)
            .fetch_optional(pool)
            .await?
            .is_some();
        if !meeting_exists {
            log_info!(
                "upsert_prep_notes: meeting_id {} does not exist; skipping",
                meeting_id
            );
            return Ok(false);
        }

        let now = Utc::now().to_rfc3339();
        sqlx::query(
            r#"
            INSERT INTO meeting_notes (meeting_id, prep_markdown, prep_json, prep_updated_at, created_at, updated_at)
            VALUES (?, ?, ?, ?, ?, ?)
            ON CONFLICT(meeting_id) DO UPDATE SET
                prep_markdown = excluded.prep_markdown,
                prep_json = excluded.prep_json,
                prep_updated_at = excluded.prep_updated_at
            "#,
        )
        .bind(meeting_id)
        .bind(prep_markdown)
        .bind(prep_json)
        .bind(&now)
        .bind(&now)
        .bind(&now)
        .execute(pool)
        .await?;

        Ok(true)
    }

    /// Loads a meeting's notes, or None if no notes row exists yet.
    ///
    /// Note: the `enhanced_*` columns are dormant as of the spec 0003 pivot
    /// (notes-aware summary); they are still selected via `SELECT *` but no code
    /// writes them. See `migrations/20260623000000_add_enhanced_notes.sql`.
    pub async fn get_notes(
        pool: &SqlitePool,
        meeting_id: &str,
    ) -> Result<Option<MeetingNote>, sqlx::Error> {
        sqlx::query_as::<_, MeetingNote>("SELECT * FROM meeting_notes WHERE meeting_id = ?")
            .bind(meeting_id)
            .fetch_optional(pool)
            .await
    }
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

    /// specs/0036: prep notes and live notes are independent columns — writing one never
    /// clobbers the other, in either order.
    #[tokio::test]
    async fn prep_notes_and_live_notes_are_independent() {
        let pool = test_pool().await;
        let id = MeetingsRepository::create_meeting(
            &pool,
            Some("Weekly".into()),
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();

        // Prep first (no live notes row yet).
        assert!(MeetingNotesRepository::upsert_prep_notes(
            &pool,
            &id,
            Some("- cover roadmap"),
            Some("{}")
        )
        .await
        .unwrap());
        let n = MeetingNotesRepository::get_notes(&pool, &id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(n.prep_markdown.as_deref(), Some("- cover roadmap"));
        assert!(
            n.notes_markdown.is_none(),
            "prep write leaves live notes untouched"
        );

        // Live notes next — must not clobber prep.
        assert!(MeetingNotesRepository::upsert_notes(
            &pool,
            &id,
            Some("we discussed hiring"),
            Some("{}")
        )
        .await
        .unwrap());
        let n = MeetingNotesRepository::get_notes(&pool, &id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(n.notes_markdown.as_deref(), Some("we discussed hiring"));
        assert_eq!(
            n.prep_markdown.as_deref(),
            Some("- cover roadmap"),
            "live-notes write leaves prep untouched"
        );

        // Update prep — live notes survive.
        assert!(MeetingNotesRepository::upsert_prep_notes(
            &pool,
            &id,
            Some("- cover roadmap\n- OKRs"),
            None
        )
        .await
        .unwrap());
        let n = MeetingNotesRepository::get_notes(&pool, &id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(n.prep_markdown.as_deref(), Some("- cover roadmap\n- OKRs"));
        assert_eq!(n.notes_markdown.as_deref(), Some("we discussed hiring"));

        // Unknown meeting → false, no row created.
        assert!(!MeetingNotesRepository::upsert_prep_notes(
            &pool,
            "meeting-missing",
            Some("x"),
            None
        )
        .await
        .unwrap());
    }
}
