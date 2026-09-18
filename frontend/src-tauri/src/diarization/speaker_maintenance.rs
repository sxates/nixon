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
/// Idempotent AND non-destructive (specs/0061 R-fix I1): `SpeakersRepository::
/// insert_if_absent` is `INSERT ... ON CONFLICT DO NOTHING`, so calling this twice for
/// the same meeting never fails or duplicates a row, and — unlike `upsert` — never
/// resets an already-present row's `display_name`. The owner row is renamable from the
/// legend, and the diarization pipeline deliberately preserves that rename across
/// re-runs (WS3.2); using `upsert` here would silently revert a renamed owner back to
/// "You" on every reassignment of a line to `local`.
pub(crate) async fn ensure_local_speaker(
    pool: &SqlitePool,
    meeting_id: &str,
) -> Result<(), SqlxError> {
    SpeakersRepository::insert_if_absent(pool, meeting_id, LOCAL_SPEAKER_KEY, "You", true).await
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

/// The earliest transcript row id ANY of the given speaker keys currently holds in a
/// meeting (specs/0061 W4 task 3's click-to-filter jump target). Takes every member key
/// of a consolidated speaker group (controller ruling R36) — a group's true earliest
/// line can belong to a non-primary key, so the ordering must span all of them in SQL
/// (the id alone carries no timestamp for a frontend-side comparison to work with).
pub(crate) async fn first_segment_id_inner(
    pool: &SqlitePool,
    meeting_id: &str,
    speaker_keys: &[String],
) -> anyhow::Result<Option<String>> {
    Ok(SpeakersRepository::first_segment_id(pool, meeting_id, speaker_keys).await?)
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

/// The earliest transcript line id across ALL of the given speaker keys in a meeting, or
/// `None` when none of them have segments (specs/0061 W4 task 3's click-to-filter).
/// Takes every member key of a consolidated speaker group (controller ruling R36) so a
/// group whose displayed identity spans several raw diarization keys still jumps to its
/// true earliest line, not just the primary key's own earliest.
#[tauri::command]
pub async fn api_first_segment_for_speaker<R: Runtime>(
    app: AppHandle<R>,
    meeting_id: String,
    speaker_keys: Vec<String>,
) -> Result<Option<String>, String> {
    let state = app.state::<AppState>();
    let pool = state.db_manager.pool();
    first_segment_id_inner(pool, &meeting_id, &speaker_keys)
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

    /// specs/0061 R-fix I1: reassigning a line to "You" must not revert a renamed owner.
    /// The owner row is renamable from the legend and the diarization pipeline
    /// deliberately preserves such renames across re-runs (WS3.2) — `ensure_local_speaker`
    /// upserting over that rename would contradict that invariant.
    #[tokio::test]
    async fn reassigning_to_local_preserves_a_renamed_owner() {
        let pool = pool_with_schema().await;
        let m = insert_meeting(&pool).await;
        let ids = insert_lines(&pool, &m, 2).await;
        insert_speaker(&pool, &m, "local", "You").await;
        assert!(SpeakersRepository::rename(&pool, &m, "local", "Priya")
            .await
            .unwrap());

        crate::diarization::corrections::set_segment_speakers_inner(&pool, &m, ids, "local")
            .await
            .unwrap();

        let (name,): (String,) = sqlx::query_as(
            "SELECT display_name FROM speakers WHERE meeting_id = ? AND speaker_key = 'local'",
        )
        .bind(&m)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(
            name, "Priya",
            "reassigning to 'local' must not revert the rename"
        );
    }

    /// specs/0061 R-fix I1: calling `ensure_local_speaker` twice must not duplicate the row.
    #[tokio::test]
    async fn ensure_local_speaker_twice_does_not_duplicate_a_row() {
        let pool = pool_with_schema().await;
        let m = insert_meeting(&pool).await;

        ensure_local_speaker(&pool, &m).await.unwrap();
        ensure_local_speaker(&pool, &m).await.unwrap();

        let (n,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM speakers WHERE meeting_id = ? AND speaker_key = 'local'",
        )
        .bind(&m)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(n, 1);
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
            first_segment_id_inner(&pool, &m, &["spk_0".to_string()])
                .await
                .unwrap(),
            Some(ids[1].clone())
        );
    }

    /// Controller ruling R36 (specs/0061 W4 task 3 review) — a consolidated speaker
    /// group's TRUE earliest line can belong to a NON-primary member key (two raw
    /// diarization keys mapped to the same Person). A primary-key-only lookup can only
    /// ever see the primary's own rows and picks the wrong line — confirmed by running
    /// this exact scenario against the old single-key `first_segment_id_inner(&pool, &m,
    /// "spk_0")` before this fix: it returned `ids[1]` (start=1), not the group's true
    /// earliest `ids[0]` (start=0, owned by non-primary key `spk_1`). The widened,
    /// multi-key SQL lookup below orders by `audio_start_time` across every member key.
    #[tokio::test]
    async fn first_segment_id_spans_every_member_key_ordered_by_audio_start() {
        let pool = pool_with_schema().await;
        let m = insert_meeting(&pool).await;
        let ids = insert_lines(&pool, &m, 3).await; // audio_start_time 0, 1, 2 respectively
        sqlx::query("UPDATE transcripts SET speaker = 'spk_1' WHERE id = ?")
            .bind(&ids[0]) // start=0 — the group's TRUE earliest line, non-primary key
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("UPDATE transcripts SET speaker = 'spk_0' WHERE id = ?")
            .bind(&ids[1]) // start=1 — primary key's own earliest
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("UPDATE transcripts SET speaker = 'spk_0' WHERE id = ?")
            .bind(&ids[2])
            .execute(&pool)
            .await
            .unwrap();

        let result = first_segment_id_inner(&pool, &m, &["spk_0".to_string(), "spk_1".to_string()])
            .await
            .unwrap();
        assert_eq!(result, Some(ids[0].clone()));
    }

    /// specs/0061 R-fix M2: SQLite sorts NULL first in `ASC` order, so an untimed row
    /// must not be picked as "first" over a row that actually has a timestamp.
    #[tokio::test]
    async fn first_segment_id_does_not_pick_a_null_audio_start_time_first() {
        let pool = pool_with_schema().await;
        let m = insert_meeting(&pool).await;
        let ids = insert_lines(&pool, &m, 2).await;
        // ids[0] gets a NULL audio_start_time; ids[1] has a real, later timestamp.
        sqlx::query(
            "UPDATE transcripts SET speaker = 'spk_0', audio_start_time = NULL WHERE id = ?",
        )
        .bind(&ids[0])
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "UPDATE transcripts SET speaker = 'spk_0', audio_start_time = 5.0 WHERE id = ?",
        )
        .bind(&ids[1])
        .execute(&pool)
        .await
        .unwrap();

        assert_eq!(
            first_segment_id_inner(&pool, &m, &["spk_0".to_string()])
                .await
                .unwrap(),
            Some(ids[1].clone()),
            "a timed row must be chosen over an untimed one"
        );
    }

    #[tokio::test]
    async fn first_segment_id_returns_none_for_an_empty_key_list() {
        let pool = pool_with_schema().await;
        let m = insert_meeting(&pool).await;
        insert_lines(&pool, &m, 1).await;
        assert_eq!(first_segment_id_inner(&pool, &m, &[]).await.unwrap(), None);
    }

    /// Controller ruling R34 (specs/0061 W4 review): the post-reassignment prune is
    /// best-effort — a cleanup failure must never turn an already-committed correction
    /// into a reported error. Drops the `speakers` table (which `prune_empty_speakers_inner`
    /// reads/writes but the override write path never touches) so the reassignment itself
    /// still succeeds while the prune step that follows it fails; the whole call must still
    /// return `Ok`.
    #[tokio::test]
    async fn prune_failure_after_reassignment_does_not_fail_the_correction() {
        let pool = pool_with_schema().await;
        let m = insert_meeting(&pool).await;
        let ids = insert_lines(&pool, &m, 1).await;
        sqlx::query("DROP TABLE speakers")
            .execute(&pool)
            .await
            .unwrap();

        let result =
            crate::diarization::corrections::set_segment_speaker_inner(&pool, &m, &ids[0], "spk_0")
                .await;
        assert!(
            result.is_ok(),
            "a prune failure must not fail an already-committed correction: {result:?}"
        );
    }
}
