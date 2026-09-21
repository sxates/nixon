use crate::database::models::{MeetingModel, TranscriptWithSpeaker};
use crate::meetings::{MeetingDetails, MeetingTranscript};
use chrono::{DateTime, Utc};
use sqlx::{Connection, Error as SqlxError, SqliteConnection, SqlitePool};
use tracing::{error, info};
use uuid::Uuid;

use super::MeetingsRepository;

/// All transcript columns plus the LEFT-JOINed speaker `display_name`, ordered by
/// recording-relative start (untimed rows last). Used by the full-meeting read.
const SELECT_TRANSCRIPTS_WITH_SPEAKER: &str =
    "SELECT t.id, t.meeting_id, t.transcript, t.timestamp, \
            t.audio_start_time, t.audio_end_time, t.duration, t.speaker, \
            s.display_name AS speaker_name, t.user_edited \
     FROM transcripts t \
     LEFT JOIN speakers s \
       ON s.meeting_id = t.meeting_id AND s.speaker_key = t.speaker \
     WHERE t.meeting_id = ? \
     ORDER BY t.audio_start_time IS NULL, t.audio_start_time, t.timestamp";

impl MeetingsRepository {
    /// Creates a single empty `meetings` row and returns its generated id.
    ///
    /// Used by the "persist at recording START" flow (specs/0007): the frontend
    /// calls this when recording begins so notes can autosave against a real
    /// meeting id while the meeting is still in progress. Transcript segments are
    /// attached later on STOP via
    /// [`TranscriptsRepository::save_transcripts_for_meeting`].
    ///
    /// `title` falls back to "New Meeting" when `None`/empty so the row always has
    /// a usable label before the user (or the title generator) sets one.
    ///
    /// `origin` types the meeting (specs/0015): "recorded" | "notes_only" | "imported";
    /// `None` defaults to "recorded" so the existing recording-start callers are
    /// byte-compatible. `calendar_event_id` links the meeting to its EventKit calendar
    /// event (Join & Record); `None` for ad-hoc/notes-only meetings.
    ///
    /// `started_at` overrides the meeting's `created_at` (its displayed date/time).
    /// For a meeting created from a calendar event (Join & Record), pass the event's
    /// scheduled start so the meeting is dated to the event — regardless of when the
    /// recording actually began. `None` uses now (the normal ad-hoc recording case).
    /// `updated_at` is always now.
    pub async fn create_meeting(
        pool: &SqlitePool,
        title: Option<String>,
        folder_path: Option<String>,
        origin: Option<String>,
        calendar_event_id: Option<String>,
        started_at: Option<DateTime<Utc>>,
    ) -> Result<String, SqlxError> {
        let meeting_id = format!("meeting-{}", Uuid::new_v4());
        let title = title
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty())
            .unwrap_or_else(|| "New Meeting".to_string());
        let origin = origin
            .map(|o| o.trim().to_string())
            .filter(|o| !o.is_empty())
            .unwrap_or_else(|| "recorded".to_string());

        let now = Utc::now();
        let created_at = started_at.unwrap_or(now);

        sqlx::query(
            "INSERT INTO meetings (id, title, created_at, updated_at, folder_path, origin, calendar_event_id) VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&meeting_id)
        .bind(&title)
        .bind(created_at)
        .bind(now)
        .bind(&folder_path)
        .bind(&origin)
        .bind(&calendar_event_id)
        .execute(pool)
        .await?;

        info!("Created meeting at recording start with id: {}", meeting_id);
        Ok(meeting_id)
    }

    /// Find an EMPTY (no transcripts), recently-touched meeting linked to this calendar event,
    /// suitable to ADOPT instead of minting a duplicate (specs/0024 WS2.1). This is the backend
    /// safety net for the symptom where a calendar-started recording produced a titled-but-empty
    /// row PLUS a separate row holding the transcript: if the recorder issues a second
    /// calendar-linked create, we reuse the existing empty row instead.
    ///
    /// Deliberately narrow so it can NEVER fold a real prior recording:
    ///  - **empty only** — a row that already has transcripts is a genuine separate recording
    ///    (recurring events share one EventKit id across occurrences, see specs/0019 WS6.3), so
    ///    we never merge into it.
    ///  - **recent only** — `updated_at` within the last 6h (updated_at is wall-clock-at-create,
    ///    unlike created_at which is dated to the event), so a stale empty row from a previous
    ///    occurrence isn't adopted with the wrong date.
    pub async fn find_adoptable_calendar_meeting(
        pool: &SqlitePool,
        calendar_event_id: &str,
    ) -> Result<Option<String>, SqlxError> {
        if calendar_event_id.trim().is_empty() {
            return Ok(None);
        }
        let cutoff = Utc::now() - chrono::Duration::hours(6);
        let row: Option<(String,)> = sqlx::query_as(
            "SELECT m.id FROM meetings m \
             WHERE m.calendar_event_id = ? \
               AND m.updated_at >= ? \
               AND NOT EXISTS (SELECT 1 FROM transcripts t WHERE t.meeting_id = m.id) \
             ORDER BY m.updated_at DESC LIMIT 1",
        )
        .bind(calendar_event_id)
        .bind(cutoff)
        .fetch_optional(pool)
        .await?;
        Ok(row.map(|(id,)| id))
    }

    pub async fn delete_meeting(pool: &SqlitePool, meeting_id: &str) -> Result<bool, SqlxError> {
        if meeting_id.trim().is_empty() {
            return Err(SqlxError::Protocol(
                "meeting_id cannot be empty".to_string(),
            ));
        }

        let mut conn = pool.acquire().await?;
        let mut transaction = conn.begin().await?;

        match delete_meeting_with_transaction(&mut transaction, meeting_id).await {
            Ok(success) => {
                if success {
                    transaction.commit().await?;
                    info!(
                        "Successfully deleted meeting {} and all associated data",
                        meeting_id
                    );
                    Ok(true)
                } else {
                    transaction.rollback().await?;
                    Ok(false)
                }
            }
            Err(e) => {
                let _ = transaction.rollback().await;
                error!("Failed to delete meeting {}: {}", meeting_id, e);
                Err(e)
            }
        }
    }

    pub async fn get_meeting(
        pool: &SqlitePool,
        meeting_id: &str,
    ) -> Result<Option<MeetingDetails>, SqlxError> {
        if meeting_id.trim().is_empty() {
            return Err(SqlxError::Protocol(
                "meeting_id cannot be empty".to_string(),
            ));
        }

        let mut conn = pool.acquire().await?;
        let mut transaction = conn.begin().await?;

        // Get meeting details
        let meeting: Option<MeetingModel> =
            sqlx::query_as("SELECT id, title, created_at, updated_at, folder_path, origin, calendar_event_id, title_manually_set, template_id FROM meetings WHERE id = ?")
                .bind(meeting_id)
                .fetch_optional(&mut *transaction)
                .await?;

        if meeting.is_none() {
            transaction.rollback().await?;
            return Err(SqlxError::RowNotFound);
        }

        if let Some(meeting) = meeting {
            // Get all transcripts for this meeting, LEFT JOINing the per-meeting
            // speaker row so callers get both the stable `speaker` key and the
            // resolved `display_name` (NULL for un-diarized meetings — unchanged).
            let transcripts =
                sqlx::query_as::<_, TranscriptWithSpeaker>(SELECT_TRANSCRIPTS_WITH_SPEAKER)
                    .bind(meeting_id)
                    .fetch_all(&mut *transaction)
                    .await?;

            transaction.commit().await?;

            // Convert to MeetingTranscript
            let meeting_transcripts = transcripts
                .into_iter()
                .map(MeetingTranscript::from)
                .collect::<Vec<_>>();

            Ok(Some(MeetingDetails {
                id: meeting.id,
                title: meeting.title,
                created_at: meeting.created_at.0.to_rfc3339(),
                updated_at: meeting.updated_at.0.to_rfc3339(),
                origin: meeting.origin,
                calendar_event_id: meeting.calendar_event_id,
                transcripts: meeting_transcripts,
            }))
        } else {
            transaction.rollback().await?;
            Ok(None)
        }
    }

    /// Get meeting metadata without transcripts (for pagination)
    pub async fn get_meeting_metadata(
        pool: &SqlitePool,
        meeting_id: &str,
    ) -> Result<Option<MeetingModel>, SqlxError> {
        if meeting_id.trim().is_empty() {
            return Err(SqlxError::Protocol(
                "meeting_id cannot be empty".to_string(),
            ));
        }

        let meeting: Option<MeetingModel> =
            sqlx::query_as("SELECT id, title, created_at, updated_at, folder_path, origin, calendar_event_id, title_manually_set, template_id, calendar_series_key, processing_mode, scheduled_end_at, join_url FROM meetings WHERE id = ?")
                .bind(meeting_id)
                .fetch_optional(pool)
                .await?;

        Ok(meeting)
    }

    /// Backfill `meetings.folder_path` for a meeting whose folder was never persisted
    /// (the frontend save can race the folder write). Best-effort: callers log and
    /// continue on failure. Returns whether a row was updated.
    pub async fn update_folder_path(
        pool: &SqlitePool,
        meeting_id: &str,
        folder_path: &str,
    ) -> Result<bool, SqlxError> {
        if meeting_id.trim().is_empty() {
            return Err(SqlxError::Protocol(
                "meeting_id cannot be empty".to_string(),
            ));
        }

        let result = sqlx::query("UPDATE meetings SET folder_path = ? WHERE id = ?")
            .bind(folder_path)
            .bind(meeting_id)
            .execute(pool)
            .await?;

        Ok(result.rows_affected() > 0)
    }

    /// Reads the persisted per-meeting summary template id (specs/0029 WS4.3 — the
    /// specs/0020 `meetings.template_id` persistence slice). Returns `None` both when
    /// the meeting has no explicit choice (NULL = "use the default template") and when
    /// the meeting doesn't exist — callers that need existence use the setter's bool.
    pub async fn get_meeting_template(
        pool: &SqlitePool,
        meeting_id: &str,
    ) -> Result<Option<String>, SqlxError> {
        if meeting_id.trim().is_empty() {
            return Err(SqlxError::Protocol(
                "meeting_id cannot be empty".to_string(),
            ));
        }

        let row: Option<(Option<String>,)> =
            sqlx::query_as("SELECT template_id FROM meetings WHERE id = ?")
                .bind(meeting_id)
                .fetch_optional(pool)
                .await?;

        Ok(row.and_then(|(template_id,)| template_id))
    }

    /// Persists the summary template for one meeting. `None` (or a blank string,
    /// normalized here) clears the choice back to NULL = "use the default template".
    /// Returns whether a row was updated (false ⇒ meeting not found). Deliberately
    /// does not bump `updated_at` (mirrors `update_folder_path`): picking a template
    /// is metadata, not a content edit.
    pub async fn set_meeting_template(
        pool: &SqlitePool,
        meeting_id: &str,
        template_id: Option<&str>,
    ) -> Result<bool, SqlxError> {
        if meeting_id.trim().is_empty() {
            return Err(SqlxError::Protocol(
                "meeting_id cannot be empty".to_string(),
            ));
        }

        let template_id = template_id.map(str::trim).filter(|t| !t.is_empty());

        let result = sqlx::query("UPDATE meetings SET template_id = ? WHERE id = ?")
            .bind(template_id)
            .bind(meeting_id)
            .execute(pool)
            .await?;

        Ok(result.rows_affected() > 0)
    }

    /// Reads the per-meeting processing-mode override (low-power-mode spec §3).
    /// NULL/absent → None → "follow the global decision".
    pub async fn get_processing_mode(
        pool: &SqlitePool,
        meeting_id: &str,
    ) -> Result<Option<String>, SqlxError> {
        if meeting_id.trim().is_empty() {
            return Err(SqlxError::Protocol(
                "meeting_id cannot be empty".to_string(),
            ));
        }
        let row: Option<(Option<String>,)> =
            sqlx::query_as("SELECT processing_mode FROM meetings WHERE id = ?")
                .bind(meeting_id)
                .fetch_optional(pool)
                .await?;
        Ok(row.and_then(|(mode,)| mode))
    }

    /// Persists the processing-mode override. `None`/blank clears to NULL.
    /// Only 'live' and 'defer' are accepted. Returns whether a row was updated
    /// (false ⇒ meeting not found). Does not bump `updated_at` (metadata, not
    /// content — mirrors `set_meeting_template`).
    pub async fn set_processing_mode(
        pool: &SqlitePool,
        meeting_id: &str,
        mode: Option<&str>,
    ) -> Result<bool, SqlxError> {
        if meeting_id.trim().is_empty() {
            return Err(SqlxError::Protocol(
                "meeting_id cannot be empty".to_string(),
            ));
        }
        let mode = mode.map(str::trim).filter(|m| !m.is_empty());
        if let Some(m) = mode {
            if m != crate::audio::processing_mode::MODE_LIVE
                && m != crate::audio::processing_mode::MODE_DEFER
            {
                return Err(SqlxError::Protocol(format!(
                    "invalid processing_mode {m:?} (expected 'live' or 'defer')"
                )));
            }
        }
        let result = sqlx::query("UPDATE meetings SET processing_mode = ? WHERE id = ?")
            .bind(mode)
            .bind(meeting_id)
            .execute(pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn update_meeting_title(
        pool: &SqlitePool,
        meeting_id: &str,
        new_title: &str,
    ) -> Result<bool, SqlxError> {
        if meeting_id.trim().is_empty() {
            return Err(SqlxError::Protocol(
                "meeting_id cannot be empty".to_string(),
            ));
        }

        let mut conn = pool.acquire().await?;
        let mut transaction = conn.begin().await?;

        let now = Utc::now().naive_utc();

        // This is the MANUAL rename path (api_save_meeting_title). Mark title_manually_set so
        // summary auto-titling never overwrites a name the user chose (specs/0024 WS6.1). The
        // AI rename uses update_meeting_name, which deliberately does NOT touch this flag.
        let rows_affected = sqlx::query(
            "UPDATE meetings SET title = ?, title_manually_set = 1, updated_at = ? WHERE id = ?",
        )
        .bind(new_title)
        .bind(now)
        .bind(meeting_id)
        .execute(&mut *transaction)
        .await?;
        if rows_affected.rows_affected() == 0 {
            transaction.rollback().await?;
            return Ok(false);
        }
        transaction.commit().await?;
        Ok(true)
    }

    pub async fn update_meeting_name(
        pool: &SqlitePool,
        meeting_id: &str,
        new_title: &str,
    ) -> Result<bool, SqlxError> {
        let mut transaction = pool.begin().await?;
        let now = Utc::now();

        // Update meetings table
        let meeting_update =
            sqlx::query("UPDATE meetings SET title = ?, updated_at = ? WHERE id = ?")
                .bind(new_title)
                .bind(now)
                .bind(meeting_id)
                .execute(&mut *transaction)
                .await?;

        if meeting_update.rows_affected() == 0 {
            transaction.rollback().await?;
            return Ok(false); // Meeting not found
        }

        // Update transcript_chunks table
        sqlx::query("UPDATE transcript_chunks SET meeting_name = ? WHERE meeting_id = ?")
            .bind(new_title)
            .bind(meeting_id)
            .execute(&mut *transaction)
            .await?;

        transaction.commit().await?;
        Ok(true)
    }
}

async fn delete_meeting_with_transaction(
    transaction: &mut SqliteConnection,
    meeting_id: &str,
) -> Result<bool, SqlxError> {
    // Check if meeting exists
    let meeting_exists: Option<(i64,)> = sqlx::query_as("SELECT 1 FROM meetings WHERE id = ?")
        .bind(meeting_id)
        .fetch_optional(&mut *transaction)
        .await?;

    if meeting_exists.is_none() {
        error!("Meeting {} not found for deletion", meeting_id);
        return Ok(false);
    }

    // Delete from related tables in proper order (children before parent).
    // NOTE (specs/0028): the pool now connects with `foreign_keys(true)`, so the
    // declared ON DELETE CASCADE FKs are enforced. This explicit children-first
    // teardown remains correct and non-double-deleting: each child DELETE simply
    // affects the rows before the parent is removed, and the final `DELETE FROM
    // meetings` cascade then finds nothing left to cascade. Tables whose FKs are
    // doc-only (no ON DELETE CASCADE) still REQUIRE these explicit deletes.
    // 1. Delete from transcript_chunks
    sqlx::query("DELETE FROM transcript_chunks WHERE meeting_id = ?")
        .bind(meeting_id)
        .execute(&mut *transaction)
        .await?;

    // 2. Delete from summary_processes
    sqlx::query("DELETE FROM summary_processes WHERE meeting_id = ?")
        .bind(meeting_id)
        .execute(&mut *transaction)
        .await?;

    // 3. Delete from transcripts
    sqlx::query("DELETE FROM transcripts WHERE meeting_id = ?")
        .bind(meeting_id)
        .execute(&mut *transaction)
        .await?;

    // 4. Delete from meeting_notes (user + AI-enhanced notes). FK is ON DELETE
    // CASCADE and now enforced; deleting explicitly (children-first) is still safe.
    sqlx::query("DELETE FROM meeting_notes WHERE meeting_id = ?")
        .bind(meeting_id)
        .execute(&mut *transaction)
        .await?;

    // 4b. Delete the cached pre-call-prep brief (specs/0036). FK is ON DELETE CASCADE;
    // explicit children-first delete mirrors the meeting_notes handling.
    sqlx::query("DELETE FROM meeting_briefs WHERE meeting_id = ?")
        .bind(meeting_id)
        .execute(&mut *transaction)
        .await?;

    // 5. Delete diarization speakers (specs/0010). FK is ON DELETE CASCADE and now
    // enforced; explicit children-first delete remains safe.
    sqlx::query("DELETE FROM speakers WHERE meeting_id = ?")
        .bind(meeting_id)
        .execute(&mut *transaction)
        .await?;

    // 6. Delete the participant roster (specs/0017). FK is doc-only (no `PRAGMA
    // foreign_keys`), so delete explicitly — mirrors the speakers/meeting_notes handling.
    // This removes only the join rows; the People they reference outlive the meeting.
    sqlx::query("DELETE FROM meeting_participants WHERE meeting_id = ?")
        .bind(meeting_id)
        .execute(&mut *transaction)
        .await?;

    // 7. Delete sticky per-segment speaker overrides (specs/0019 WS2.3). FK is doc-only,
    // so delete explicitly to avoid orphaned override rows after the transcripts are gone.
    sqlx::query("DELETE FROM transcript_speaker_overrides WHERE meeting_id = ?")
        .bind(meeting_id)
        .execute(&mut *transaction)
        .await?;

    // 8. Delete action items + their extraction ledger row (specs/0034). FKs are doc-only,
    // so delete explicitly. Standalone to-dos (meeting_id IS NULL) are untouched.
    sqlx::query("DELETE FROM action_items WHERE meeting_id = ?")
        .bind(meeting_id)
        .execute(&mut *transaction)
        .await?;
    sqlx::query("DELETE FROM action_item_extractions WHERE meeting_id = ?")
        .bind(meeting_id)
        .execute(&mut *transaction)
        .await?;

    // 9. Delete the manual series link (specs/0041 WS4). FK is ON DELETE CASCADE and
    // enforced; explicit children-first delete mirrors the meeting_notes handling.
    sqlx::query("DELETE FROM meeting_series_links WHERE meeting_id = ?")
        .bind(meeting_id)
        .execute(&mut *transaction)
        .await?;

    // 10. Finally, delete the meeting
    let result = sqlx::query("DELETE FROM meetings WHERE id = ?")
        .bind(meeting_id)
        .execute(&mut *transaction)
        .await?;

    Ok(result.rows_affected() > 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::repositories::meeting::test_support::{dt, memory_db};

    /// specs/0069b review fix 2 — `get_meeting_metadata` (the `MeetingModel` read that
    /// backs `MeetingMetadata`, the DTO the meeting-details page actually renders from)
    /// must carry `scheduled_end_at`/`join_url`, and `calendar_event_id` must classify
    /// correctly under `is_manual_event_id` (what `api_get_meeting_metadata` uses to
    /// compute `is_manual_entry`): `true` for a manual entry, `false` for a
    /// calendar-backed scheduled row and for a plain recorded meeting.
    #[tokio::test]
    async fn get_meeting_metadata_reports_manual_entry_and_its_schedule_fields() {
        let pool = memory_db().await;

        let manual_id = MeetingsRepository::create_manual_scheduled(
            &pool,
            "Call with Sam",
            dt("2026-09-20T15:00:00Z"),
            Some(dt("2026-09-20T15:30:00Z")),
            Some("https://zoom.us/j/1"),
        )
        .await
        .unwrap();
        let manual = MeetingsRepository::get_meeting_metadata(&pool, &manual_id)
            .await
            .unwrap()
            .expect("manual meeting exists");
        assert!(crate::database::repositories::meeting::is_manual_event_id(
            manual.calendar_event_id.as_deref().unwrap_or("")
        ));
        assert_eq!(manual.join_url.as_deref(), Some("https://zoom.us/j/1"));
        assert!(manual.scheduled_end_at.is_some());

        sqlx::query(
            "INSERT INTO meetings (id, title, created_at, updated_at, origin, calendar_event_id) \
             VALUES ('m-cal', 'From the calendar', ?1, ?1, 'scheduled', 'gcal:cal/evt_1')",
        )
        .bind(dt("2026-09-20T10:00:00Z"))
        .execute(&pool)
        .await
        .unwrap();
        let calendar_scheduled = MeetingsRepository::get_meeting_metadata(&pool, "m-cal")
            .await
            .unwrap()
            .expect("calendar-backed meeting exists");
        assert!(
            !crate::database::repositories::meeting::is_manual_event_id(
                calendar_scheduled.calendar_event_id.as_deref().unwrap_or("")
            ),
            "a calendar-backed scheduled row is not a manual entry"
        );
        assert_eq!(calendar_scheduled.join_url, None);
        assert!(calendar_scheduled.scheduled_end_at.is_none());

        let recorded_id = MeetingsRepository::create_meeting(&pool, None, None, None, None, None)
            .await
            .unwrap();
        let recorded = MeetingsRepository::get_meeting_metadata(&pool, &recorded_id)
            .await
            .unwrap()
            .expect("recorded meeting exists");
        assert!(recorded.calendar_event_id.is_none());
    }

    /// specs/0029 WS4.3: per-meeting template persistence (the specs/0020 slice).
    #[tokio::test]
    async fn meeting_template_get_set_roundtrip() {
        let pool = memory_db().await;
        let id = MeetingsRepository::create_meeting(&pool, None, None, None, None, None)
            .await
            .expect("create_meeting");

        // Fresh meeting: no explicit template (NULL = "use the default").
        assert_eq!(
            MeetingsRepository::get_meeting_template(&pool, &id)
                .await
                .unwrap(),
            None
        );

        // Set + read back.
        assert!(
            MeetingsRepository::set_meeting_template(&pool, &id, Some("daily_standup"))
                .await
                .unwrap()
        );
        assert_eq!(
            MeetingsRepository::get_meeting_template(&pool, &id)
                .await
                .unwrap(),
            Some("daily_standup".to_string())
        );

        // Overwrite with a new choice.
        assert!(
            MeetingsRepository::set_meeting_template(&pool, &id, Some("standard_meeting"))
                .await
                .unwrap()
        );
        assert_eq!(
            MeetingsRepository::get_meeting_template(&pool, &id)
                .await
                .unwrap(),
            Some("standard_meeting".to_string())
        );

        // The metadata read (MeetingModel) carries the column too.
        let meta = MeetingsRepository::get_meeting_metadata(&pool, &id)
            .await
            .unwrap()
            .expect("meeting exists");
        assert_eq!(meta.template_id.as_deref(), Some("standard_meeting"));

        // Clear back to default; whitespace-only normalizes to a clear as well.
        assert!(MeetingsRepository::set_meeting_template(&pool, &id, None)
            .await
            .unwrap());
        assert_eq!(
            MeetingsRepository::get_meeting_template(&pool, &id)
                .await
                .unwrap(),
            None
        );
        assert!(
            MeetingsRepository::set_meeting_template(&pool, &id, Some("  "))
                .await
                .unwrap()
        );
        assert_eq!(
            MeetingsRepository::get_meeting_template(&pool, &id)
                .await
                .unwrap(),
            None
        );
    }

    #[tokio::test]
    async fn meeting_template_missing_meeting_and_bad_input() {
        let pool = memory_db().await;

        // Unknown meeting: set reports "no row updated", get reports None.
        assert!(
            !MeetingsRepository::set_meeting_template(&pool, "meeting-missing", Some("x"))
                .await
                .unwrap()
        );
        assert_eq!(
            MeetingsRepository::get_meeting_template(&pool, "meeting-missing")
                .await
                .unwrap(),
            None
        );

        // Empty meeting_id is rejected outright (matches the other repo methods).
        assert!(MeetingsRepository::set_meeting_template(&pool, "  ", None)
            .await
            .is_err());
        assert!(MeetingsRepository::get_meeting_template(&pool, "")
            .await
            .is_err());
    }
}
