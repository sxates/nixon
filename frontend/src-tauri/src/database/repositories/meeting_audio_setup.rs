//! `meetings.audio_setup` / `meetings.audio_setup_resolved` access (specs/0078).
//!
//! The override is what the user chose in "Who was on the mic?"; the resolved value is
//! what the last diarization pass used. Both live on the `meetings` row; the DB is the
//! source of truth (the recording folder's `metadata.json` stays the capture record).

use sqlx::SqlitePool;

use crate::diarization::room_types::{AudioSetup, AudioSetupOverride};

/// A meeting's stored audio setup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MeetingAudioSetup {
    /// The user's override; `Auto` when none is stored.
    pub override_setup: AudioSetupOverride,
    /// What the last diarization pass used; `None` before any pass since specs/0078.
    pub resolved: Option<AudioSetup>,
}

pub struct MeetingAudioSetupRepository;

impl MeetingAudioSetupRepository {
    /// The meeting's stored setup, or `None` when the meeting doesn't exist.
    pub async fn get(
        pool: &SqlitePool,
        meeting_id: &str,
    ) -> Result<Option<MeetingAudioSetup>, sqlx::Error> {
        let row: Option<(Option<String>, Option<String>)> =
            sqlx::query_as("SELECT audio_setup, audio_setup_resolved FROM meetings WHERE id = ?")
                .bind(meeting_id)
                .fetch_optional(pool)
                .await?;
        Ok(row.map(|(ovr, resolved)| MeetingAudioSetup {
            override_setup: AudioSetupOverride::from_db(ovr.as_deref()),
            resolved: resolved.as_deref().and_then(|s| {
                let parsed = AudioSetup::parse(s);
                if parsed.is_none() {
                    log::warn!("unknown meetings.audio_setup_resolved value {s:?}; ignoring");
                }
                parsed
            }),
        }))
    }

    /// Store the user's override (`Auto` clears it to NULL). `Ok(false)` when the meeting
    /// doesn't exist.
    pub async fn set_override(
        pool: &SqlitePool,
        meeting_id: &str,
        setup: AudioSetupOverride,
    ) -> Result<bool, sqlx::Error> {
        let res = sqlx::query("UPDATE meetings SET audio_setup = ? WHERE id = ?")
            .bind(setup.to_db())
            .bind(meeting_id)
            .execute(pool)
            .await?;
        Ok(res.rows_affected() > 0)
    }

    /// Record the setup a diarization pass is running with. Written at the start of the
    /// pass, before clustering. `Ok(false)` when the meeting doesn't exist.
    pub async fn set_resolved(
        pool: &SqlitePool,
        meeting_id: &str,
        setup: AudioSetup,
    ) -> Result<bool, sqlx::Error> {
        let res = sqlx::query("UPDATE meetings SET audio_setup_resolved = ? WHERE id = ?")
            .bind(setup.as_str())
            .bind(meeting_id)
            .execute(pool)
            .await?;
        Ok(res.rows_affected() > 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn pool_with_meeting(id: &str) -> SqlitePool {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        let now = chrono::Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO meetings (id, title, created_at, updated_at) VALUES (?, 't', ?, ?)",
        )
        .bind(id)
        .bind(&now)
        .bind(&now)
        .execute(&pool)
        .await
        .unwrap();
        pool
    }

    #[tokio::test]
    async fn a_fresh_meeting_is_auto_with_no_resolved_setup() {
        let pool = pool_with_meeting("m1").await;
        let got = MeetingAudioSetupRepository::get(&pool, "m1").await.unwrap();
        assert_eq!(
            got,
            Some(MeetingAudioSetup {
                override_setup: AudioSetupOverride::Auto,
                resolved: None,
            })
        );
        assert_eq!(
            MeetingAudioSetupRepository::get(&pool, "missing")
                .await
                .unwrap(),
            None
        );
    }

    #[tokio::test]
    async fn override_and_resolved_are_stored_independently() {
        let pool = pool_with_meeting("m1").await;
        assert!(
            MeetingAudioSetupRepository::set_override(&pool, "m1", AudioSetupOverride::Room)
                .await
                .unwrap()
        );
        assert!(
            MeetingAudioSetupRepository::set_resolved(&pool, "m1", AudioSetup::Hybrid)
                .await
                .unwrap()
        );
        let got = MeetingAudioSetupRepository::get(&pool, "m1")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(got.override_setup, AudioSetupOverride::Room);
        assert_eq!(got.resolved, Some(AudioSetup::Hybrid));

        // Back to auto clears the column to NULL; the resolved value is untouched.
        MeetingAudioSetupRepository::set_override(&pool, "m1", AudioSetupOverride::Auto)
            .await
            .unwrap();
        let raw: Option<String> =
            sqlx::query_scalar("SELECT audio_setup FROM meetings WHERE id = 'm1'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(raw, None, "auto is stored as NULL");
        let got = MeetingAudioSetupRepository::get(&pool, "m1")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(got.override_setup, AudioSetupOverride::Auto);
        assert_eq!(got.resolved, Some(AudioSetup::Hybrid));
    }

    #[tokio::test]
    async fn writes_to_a_missing_meeting_report_false() {
        let pool = pool_with_meeting("m1").await;
        assert!(!MeetingAudioSetupRepository::set_override(
            &pool,
            "nope",
            AudioSetupOverride::Call
        )
        .await
        .unwrap());
        assert!(
            !MeetingAudioSetupRepository::set_resolved(&pool, "nope", AudioSetup::Room)
                .await
                .unwrap()
        );
    }

    /// The `MeetingModel` read path (`SELECT *`) sees the new columns.
    #[tokio::test]
    async fn meeting_model_carries_both_columns() {
        let pool = pool_with_meeting("m1").await;
        MeetingAudioSetupRepository::set_override(&pool, "m1", AudioSetupOverride::Call)
            .await
            .unwrap();
        MeetingAudioSetupRepository::set_resolved(&pool, "m1", AudioSetup::Room)
            .await
            .unwrap();
        let m: crate::database::models::MeetingModel =
            sqlx::query_as("SELECT * FROM meetings WHERE id = 'm1'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(m.audio_setup.as_deref(), Some("call"));
        assert_eq!(m.audio_setup_resolved.as_deref(), Some("room"));
    }
}
