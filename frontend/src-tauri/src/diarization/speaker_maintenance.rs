//! Owner-always-assignable + empty-speaker pruning (specs/0061 W4).
//!
//! Two gaps the first external user hit: (1) a transcript line could be reassigned to
//! ANY other speaker but not to the owner ("You") — no `local` `speakers` row exists
//! unless a mic-tagged segment produced one, and the correction commands only ever
//! upserted `spk_N`/`manual_*` keys. [`ensure_local_speaker`] upserts the `local` row on
//! demand the moment a correction targets it. (2) Reassigning every line away from a
//! speaker left an empty ghost row in the panel forever. [`prune_empty_speakers_inner`]
//! deletes any non-owner speaker with zero remaining transcript rows AND no stored
//! voiceprint (a voiceprint-backed row is never pruned — deleting it would orphan
//! biometric provenance). [`first_segment_id_inner`] backs the click-to-filter jump
//! (specs/0061 W4 task 3).

use sqlx::{Error as SqlxError, SqlitePool};
use tauri::{AppHandle, Manager, Runtime};

use crate::database::repositories::speaker::SpeakersRepository;
use crate::diarization::LOCAL_SPEAKER_KEY;
use crate::state::AppState;

/// Ensure the owner's `local`/"You" speaker row exists for a meeting (specs/0061 W4).
/// Idempotent: `SpeakersRepository::upsert` is `INSERT ... ON CONFLICT DO UPDATE`, so
/// calling this twice for the same meeting never fails or duplicates a row — the second
/// call just refreshes `display_name`/`is_local`/`updated_at` on the existing row.
pub(crate) async fn ensure_local_speaker(
    pool: &SqlitePool,
    meeting_id: &str,
) -> Result<(), SqlxError> {
    SpeakersRepository::upsert(
        pool,
        meeting_id,
        LOCAL_SPEAKER_KEY,
        "You",
        true,
        None,
        None,
        None,
    )
    .await
}

/// Delete every `speakers` row for a meeting that is genuinely empty: zero transcript
/// rows, not the owner (`local`), and no stored voiceprint (specs/0061 W4) — deleting a
/// voiceprint-backed row would orphan biometric data, so such a row is never pruned even
/// if it currently has no transcript rows. Returns the deleted keys; logs each one at info.
pub(crate) async fn prune_empty_speakers_inner(
    pool: &SqlitePool,
    meeting_id: &str,
) -> anyhow::Result<Vec<String>> {
    let speakers = SpeakersRepository::get_by_meeting(pool, meeting_id).await?;
    let counts = SpeakersRepository::segment_counts(pool, meeting_id).await?;

    let mut removed = Vec::new();
    for speaker in speakers {
        if speaker.is_local != 0 {
            continue;
        }
        if counts.get(&speaker.speaker_key).copied().unwrap_or(0) > 0 {
            continue;
        }
        if SpeakersRepository::has_voiceprint(pool, meeting_id, &speaker.speaker_key).await? {
            continue;
        }
        if SpeakersRepository::delete(pool, meeting_id, &speaker.speaker_key).await? {
            log::info!(
                "prune_empty_speakers: deleted empty speaker '{}' in meeting {meeting_id}",
                speaker.speaker_key
            );
            removed.push(speaker.speaker_key);
        }
    }
    Ok(removed)
}

/// The earliest transcript row id a speaker currently holds in a meeting (specs/0061 W4
/// task 3's click-to-filter jump target).
pub(crate) async fn first_segment_id_inner(
    pool: &SqlitePool,
    meeting_id: &str,
    speaker_key: &str,
) -> anyhow::Result<Option<String>> {
    Ok(SpeakersRepository::first_segment_id(pool, meeting_id, speaker_key).await?)
}

/// Delete every empty speaker for a meeting (specs/0061 W4). The panel calls this after
/// any reassignment that might have emptied a speaker out; the correction commands and
/// the merge command already call the `_inner` form directly, so this command exists for
/// callers that only have a meeting id (e.g. a manual "tidy up" affordance). Returns the
/// deleted keys.
#[tauri::command]
pub async fn api_prune_empty_speakers<R: Runtime>(
    app: AppHandle<R>,
    meeting_id: String,
) -> Result<Vec<String>, String> {
    let state = app.state::<AppState>();
    let pool = state.db_manager.pool();
    prune_empty_speakers_inner(pool, &meeting_id)
        .await
        .map_err(|e| format!("Failed to prune empty speakers: {e}"))
}

/// The earliest transcript line id for a speaker in a meeting, or `None` when the speaker
/// has no segments (specs/0061 W4 task 3's click-to-filter).
#[tauri::command]
pub async fn api_first_segment_for_speaker<R: Runtime>(
    app: AppHandle<R>,
    meeting_id: String,
    speaker_key: String,
) -> Result<Option<String>, String> {
    let state = app.state::<AppState>();
    let pool = state.db_manager.pool();
    first_segment_id_inner(pool, &meeting_id, &speaker_key)
        .await
        .map_err(|e| format!("Failed to look up the speaker's first segment: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::repositories::people::PeopleRepository;
    use crate::diarization::corrections::test_support::{
        insert_lines, insert_meeting, insert_speaker, pool_with_schema,
    };

    #[tokio::test]
    async fn reassigning_to_local_creates_the_you_row() {
        let pool = pool_with_schema().await;
        let m = insert_meeting(&pool).await;
        let ids = insert_lines(&pool, &m, 2).await;
        crate::diarization::corrections::set_segment_speakers_inner(&pool, &m, ids, "local")
            .await
            .unwrap();
        let (name, is_local): (String, i64) = sqlx::query_as(
            "SELECT display_name, is_local FROM speakers WHERE meeting_id = ? AND speaker_key = 'local'",
        )
        .bind(&m)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(name, "You");
        assert_eq!(is_local, 1);
    }

    #[tokio::test]
    async fn prune_removes_only_empty_non_local_speakers_without_voiceprints() {
        let pool = pool_with_schema().await;
        let m = insert_meeting(&pool).await;
        let ids = insert_lines(&pool, &m, 2).await;
        insert_speaker(&pool, &m, "spk_0", "A").await;
        insert_speaker(&pool, &m, "spk_1", "B").await;
        insert_speaker(&pool, &m, "spk_2", "C").await;
        insert_speaker(&pool, &m, "local", "You").await;
        sqlx::query("UPDATE transcripts SET speaker = 'spk_0' WHERE id IN (?, ?)")
            .bind(&ids[0])
            .bind(&ids[1])
            .execute(&pool)
            .await
            .unwrap();
        // `voiceprints.person_id` REFERENCES `people(id)`, and the test pool has
        // `foreign_keys` ON (sqlx default) — unlike the app's real pool, which the
        // 20260628000002_add_voiceprints.sql migration documents as connecting WITHOUT
        // that pragma. So a real `people` row is required here even though production
        // never enforces it.
        let person = PeopleRepository::create(&pool, "P", None, None, None)
            .await
            .unwrap();
        sqlx::query("INSERT INTO voiceprints (id, person_id, embedding, embedding_dim, embedding_model, source_meeting_id, source_speaker_key, sample_quality, created_at) VALUES ('vp1', ?, X'00', 1, 'm', ?, 'spk_2', 1.0, '2026-01-01T00:00:00Z')")
            .bind(&person.id)
            .bind(&m)
            .execute(&pool)
            .await
            .unwrap();
        let removed = prune_empty_speakers_inner(&pool, &m).await.unwrap();
        assert_eq!(removed, vec!["spk_1".to_string()]);
        let (n,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM speakers WHERE meeting_id = ?")
            .bind(&m)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(n, 3);
    }

    #[tokio::test]
    async fn first_segment_id_is_the_earliest_by_audio_start() {
        let pool = pool_with_schema().await;
        let m = insert_meeting(&pool).await;
        let ids = insert_lines(&pool, &m, 3).await;
        for (id, t) in ids.iter().zip([30.0, 10.0, 20.0]) {
            sqlx::query(
                "UPDATE transcripts SET speaker = 'spk_0', audio_start_time = ? WHERE id = ?",
            )
            .bind(t)
            .bind(id)
            .execute(&pool)
            .await
            .unwrap();
        }
        assert_eq!(
            first_segment_id_inner(&pool, &m, "spk_0").await.unwrap(),
            Some(ids[1].clone())
        );
    }
}
