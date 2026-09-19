//! `voiceprints` table access — the durable, best-N voice gallery (specs/0016 Phase 1c,
//! ADR-0007-gated).
//!
//! A `voiceprints` row is one L2-normalized CAM++ speaker embedding attached to a
//! `people` row. This is the ONLY durable biometric artifact in Nixon. The design
//! (ADR-0007 §4):
//!
//! - **Best-N per person+model**, not a single stored centroid: robust to drift, keeps
//!   per-sample provenance for deletion/tuning. We cap at [`MAX_SAMPLES_PER_PERSON`] and
//!   **prune on insert**, keeping the highest-`sample_quality` samples.
//! - **Centroid computed on READ** ([`centroid_for_person`](VoiceprintsRepository::centroid_for_person)):
//!   mean of the kept samples, re-L2-normalized via `embedding.rs`. Never stored.
//! - **Same-model only**: every read is scoped to an `embedding_model` so the matcher
//!   never compares cosine across incomparable vector spaces.
//!
//! Privacy invariants (ADR-0007 §1): embeddings live ONLY here, are NEVER serialized to
//! the frontend or any LLM/network payload, and are deleted explicitly (the pool runs
//! without `PRAGMA foreign_keys`, so the declared cascade does not fire — see
//! [`PeopleRepository::delete`](crate::database::repositories::people::PeopleRepository::delete)).
//!
//! Mirrors `speaker.rs`/`people.rs`: returns `SqlxError`; the command/caller layer maps
//! to user-facing strings.

use chrono::Utc;
use sqlx::{Error as SqlxError, SqlitePool};
use uuid::Uuid;

use crate::diarization::embedding::{embedding_from_bytes, embedding_to_bytes, l2_normalize};

/// Maximum stored samples per person *per embedding model* (ADR-0007 §4 best-N). On
/// insert we prune the weakest by `sample_quality` so the gallery stays bounded and
/// improves as better samples accrue. ~10 balances drift-coverage against storage.
pub const MAX_SAMPLES_PER_PERSON: usize = 10;

/// One voiceprint sample's display-safe metadata (specs/0039 WS3) — the row shape the
/// People-detail per-sample list is built from. Carries provenance + status ONLY; the
/// embedding bytes are deliberately absent (ADR-0007 §1 / specs/0016 no-egress: biometric
/// vectors never leave the `voiceprints` table).
#[derive(Debug, Clone)]
pub struct VoiceprintSample {
    /// `vp-<uuid>` primary key — the handle for quarantine/restore/delete.
    pub id: String,
    /// The meeting the sample was enrolled from (nullable — meeting may have been deleted).
    pub source_meeting_id: Option<String>,
    /// The diarizer cluster key it came from; NULL for legacy pre-0039 samples.
    pub source_speaker_key: Option<String>,
    pub created_at: String,
    pub sample_quality: Option<f32>,
    /// `true` when `quarantined_at` is set (excluded from matching, restorable).
    pub quarantined: bool,
}

/// Internal row shape for [`VoiceprintsRepository::list_for_person`] — `quarantined` is the
/// SQL-resolved `quarantined_at IS NOT NULL` flag (0/1) so the query maps cleanly.
#[derive(sqlx::FromRow)]
struct VoiceprintSampleRow {
    id: String,
    source_meeting_id: Option<String>,
    source_speaker_key: Option<String>,
    created_at: String,
    sample_quality: Option<f64>,
    quarantined: i64,
}

pub struct VoiceprintsRepository;

impl VoiceprintsRepository {
    /// Store one voiceprint sample for a person, then prune to the best
    /// [`MAX_SAMPLES_PER_PERSON`] for that `(person_id, embedding_model)` by
    /// `sample_quality` (NULL quality sorts last, so a quality-tagged sample always
    /// survives over an untagged one). The embedding is re-L2-normalized defensively so
    /// the centroid-on-read math stays well-conditioned.
    ///
    /// **Quarantined rows are exempt from the cap (specs/0039 WS3):** they are neither
    /// eviction candidates nor counted toward [`MAX_SAMPLES_PER_PERSON`]. A quarantined
    /// sample is a retraction awaiting the user's Undo (`api_restore_voiceprint_sample`);
    /// letting the prune hard-delete it would break the undo invariant (Undo would find
    /// nothing). Only LIVE (`quarantined_at IS NULL`) rows compete for the best-N slots.
    ///
    /// Runs in one transaction: insert + prune are atomic, so a concurrent read never
    /// sees the table transiently over the cap.
    pub async fn add_sample(
        pool: &SqlitePool,
        person_id: &str,
        embedding: &[f32],
        embedding_model: &str,
        source_meeting_id: Option<&str>,
        source_speaker_key: Option<&str>,
        sample_quality: Option<f32>,
    ) -> Result<(), SqlxError> {
        let normalized = l2_normalize(embedding);
        let bytes = embedding_to_bytes(&normalized);
        let dim = normalized.len() as i64;
        let id = format!("vp-{}", Uuid::new_v4());
        let now = Utc::now().to_rfc3339();

        let mut tx = pool.begin().await?;

        // `source_speaker_key` (specs/0039 WS3) back-links the sample to the diarizer cluster
        // it came from, so a later WS2 span correction can retract exactly this meeting's
        // samples for that cluster. `quarantined_at` defaults NULL (live).
        sqlx::query(
            "INSERT INTO voiceprints
                (id, person_id, embedding, embedding_dim, embedding_model,
                 source_meeting_id, source_speaker_key, sample_quality, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(person_id)
        .bind(&bytes)
        .bind(dim)
        .bind(embedding_model)
        .bind(source_meeting_id)
        .bind(source_speaker_key)
        .bind(sample_quality.map(|q| q as f64))
        .bind(&now)
        .execute(&mut *tx)
        .await?;

        // Prune to best-N for this (person, model): delete every LIVE row NOT in the top-N
        // LIVE rows by quality. `sample_quality DESC` with NULLs last, then newest first as
        // tiebreak. (SQLite sorts NULL first on DESC, so coalesce to a sentinel below any real
        // quality to keep NULL-quality samples as the prune candidates.)
        //
        // specs/0039 WS3: quarantined rows are excluded on BOTH sides — the outer
        // `quarantined_at IS NULL` means a quarantined sample is never an eviction candidate,
        // and the inner filter means it doesn't consume a best-N slot. A retraction's
        // quarantined sample therefore always survives until the user Undoes (restore) or
        // explicitly deletes it.
        sqlx::query(
            "DELETE FROM voiceprints
             WHERE person_id = ? AND embedding_model = ?
               AND quarantined_at IS NULL
               AND id NOT IN (
                   SELECT id FROM voiceprints
                   WHERE person_id = ? AND embedding_model = ? AND quarantined_at IS NULL
                   ORDER BY COALESCE(sample_quality, -1e30) DESC, created_at DESC
                   LIMIT ?
               )",
        )
        .bind(person_id)
        .bind(embedding_model)
        .bind(person_id)
        .bind(embedding_model)
        .bind(MAX_SAMPLES_PER_PERSON as i64)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;
        Ok(())
    }

    /// The match centroid for a person: the mean of their stored samples for
    /// `embedding_model`, re-L2-normalized (ADR-0007 §4 centroid-on-read). Returns `None`
    /// when the person has no samples for that model. Malformed/length-mismatched blobs
    /// are skipped defensively rather than poisoning the mean.
    pub async fn centroid_for_person(
        pool: &SqlitePool,
        person_id: &str,
        embedding_model: &str,
    ) -> Result<Option<Vec<f32>>, SqlxError> {
        // specs/0039 WS3: quarantined samples keep their row (provenance) but are excluded
        // from matching, so a suspected-bad voiceprint stops polluting the centroid without
        // being lost (restorable).
        let blobs: Vec<Vec<u8>> = sqlx::query_scalar(
            "SELECT embedding FROM voiceprints
             WHERE person_id = ? AND embedding_model = ? AND quarantined_at IS NULL",
        )
        .bind(person_id)
        .bind(embedding_model)
        .fetch_all(pool)
        .await?;

        Ok(mean_centroid(&blobs))
    }

    /// Every person's gallery centroid for `embedding_model`, as
    /// `(person_id, centroid, sample_count)` — the gallery-matching candidate set (specs/0016
    /// 1c; the count feeds the 0044 WS4 trusted gate). No-sample people omitted; in-process.
    pub async fn all_centroids(
        pool: &SqlitePool,
        embedding_model: &str,
    ) -> Result<Vec<(String, Vec<f32>, usize)>, SqlxError> {
        use std::collections::BTreeMap;

        // specs/0039 WS3: exclude quarantined samples from the candidate set (they keep
        // their row but must not contribute to any person's match centroid).
        let rows: Vec<(String, Vec<u8>)> = sqlx::query_as(
            "SELECT person_id, embedding FROM voiceprints
             WHERE embedding_model = ? AND quarantined_at IS NULL",
        )
        .bind(embedding_model)
        .fetch_all(pool)
        .await?;

        // Group blobs by person (BTreeMap → deterministic order for tests/logs).
        let mut by_person: BTreeMap<String, Vec<Vec<u8>>> = BTreeMap::new();
        for (pid, blob) in rows {
            by_person.entry(pid).or_default().push(blob);
        }

        Ok(by_person
            .into_iter()
            .filter_map(|(pid, blobs)| mean_centroid(&blobs).map(|c| (pid, c, blobs.len())))
            .collect())
    }

    /// Number of stored samples for a person (across all models) — surfaced on the
    /// People detail UI ("3 voice samples").
    pub async fn count_for_person(pool: &SqlitePool, person_id: &str) -> Result<i64, SqlxError> {
        let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM voiceprints WHERE person_id = ?")
            .bind(person_id)
            .fetch_one(pool)
            .await?;
        Ok(n)
    }

    /// Delete every stored voiceprint for a person. Used by "forget this person" and by
    /// turning on per-person opt-out (ADR-0007 §6) — both call this inside their own
    /// transaction, so this variant takes a `SqliteConnection` to enlist in the caller's
    /// tx (the explicit cascade the pool's missing `PRAGMA foreign_keys` requires).
    pub async fn delete_for_person_tx(
        conn: &mut sqlx::SqliteConnection,
        person_id: &str,
    ) -> Result<u64, SqlxError> {
        let res = sqlx::query("DELETE FROM voiceprints WHERE person_id = ?")
            .bind(person_id)
            .execute(&mut *conn)
            .await?;
        Ok(res.rows_affected())
    }

    /// Delete every stored voiceprint for a person, on the pool (its own transaction).
    /// Convenience for callers without an open tx.
    pub async fn delete_for_person(pool: &SqlitePool, person_id: &str) -> Result<u64, SqlxError> {
        let res = sqlx::query("DELETE FROM voiceprints WHERE person_id = ?")
            .bind(person_id)
            .execute(pool)
            .await?;
        Ok(res.rows_affected())
    }

    /// The per-sample gallery listing for a person (specs/0039 WS3), NEWEST first — the
    /// input to the People-detail voiceprint list where the user can quarantine/delete one
    /// bad sample. Returns provenance + status metadata ONLY: **never** the embedding bytes
    /// (the ADR-0007 §1 / specs/0016 no-egress invariant — biometric vectors never leave
    /// this table). Includes quarantined rows (flagged) so the UI can offer "restore".
    pub async fn list_for_person(
        pool: &SqlitePool,
        person_id: &str,
    ) -> Result<Vec<VoiceprintSample>, SqlxError> {
        // `quarantined_at IS NOT NULL` is resolved to the bool flag in SQL so the row maps to
        // a simple, non-complex tuple.
        let rows: Vec<VoiceprintSampleRow> = sqlx::query_as(
            "SELECT id, source_meeting_id, source_speaker_key, created_at, sample_quality,
                    (quarantined_at IS NOT NULL) AS quarantined
             FROM voiceprints
             WHERE person_id = ?
             ORDER BY created_at DESC",
        )
        .bind(person_id)
        .fetch_all(pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|r| VoiceprintSample {
                id: r.id,
                source_meeting_id: r.source_meeting_id,
                source_speaker_key: r.source_speaker_key,
                created_at: r.created_at,
                sample_quality: r.sample_quality.map(|q| q as f32),
                quarantined: r.quarantined != 0,
            })
            .collect())
    }

    /// Hard-delete one voiceprint sample by id (specs/0039 WS3) — the explicit, unrecoverable
    /// per-sample purge (distinct from [`quarantine_sample`](Self::quarantine_sample), which is
    /// the recoverable default). Returns whether a row was removed.
    pub async fn delete_sample(pool: &SqlitePool, sample_id: &str) -> Result<bool, SqlxError> {
        let res = sqlx::query("DELETE FROM voiceprints WHERE id = ?")
            .bind(sample_id)
            .execute(pool)
            .await?;
        Ok(res.rows_affected() > 0)
    }

    /// Quarantine (soft-delete) one voiceprint sample by id (specs/0039 WS3): stamp
    /// `quarantined_at` so it drops out of `centroid_for_person` / `all_centroids` matching
    /// while its row + provenance survive. Idempotent on an already-quarantined row
    /// (re-stamps the time). Returns whether a row matched.
    pub async fn quarantine_sample(pool: &SqlitePool, sample_id: &str) -> Result<bool, SqlxError> {
        let now = Utc::now().to_rfc3339();
        let res = sqlx::query("UPDATE voiceprints SET quarantined_at = ? WHERE id = ?")
            .bind(&now)
            .bind(sample_id)
            .execute(pool)
            .await?;
        Ok(res.rows_affected() > 0)
    }

    /// Restore a quarantined sample by id (specs/0039 WS3): clear `quarantined_at` so it
    /// re-enters matching — the undo for an over-eager quarantine/retraction. Idempotent on a
    /// live row. Returns whether a row matched.
    pub async fn restore_sample(pool: &SqlitePool, sample_id: &str) -> Result<bool, SqlxError> {
        let res = sqlx::query("UPDATE voiceprints SET quarantined_at = NULL WHERE id = ?")
            .bind(sample_id)
            .execute(pool)
            .await?;
        Ok(res.rows_affected() > 0)
    }

    /// Quarantine every LIVE voiceprint sample enrolled from one diarizer cluster in one
    /// meeting (specs/0039 WS3 retraction) — `(source_meeting_id, source_speaker_key)` — and
    /// return the `(sample_id, person_id)` of each row quarantined. `exclude_person_id` is the
    /// owner ("You") id: the owner gallery is NEVER touched by a remote correction (WS3 §4), so
    /// its samples are skipped. Legacy samples with a NULL `source_speaker_key` never match here
    /// (they predate the back-link) — retract those by sample id via the per-sample UI instead.
    pub async fn quarantine_for_cluster(
        pool: &SqlitePool,
        source_meeting_id: &str,
        source_speaker_key: &str,
        exclude_person_id: &str,
    ) -> Result<Vec<(String, String)>, SqlxError> {
        // REVIEW(0039): source_speaker_key is a per-run cluster label; unstable across
        // re-diarization — see morning report. (Retraction keys on the ephemeral cluster
        // label, so after a re-diarization the label space shifts and a later retraction
        // could target the wrong person. Proper fix keys on stable line ids / identity.)
        //
        // Read the live matches first so we can return exactly which rows we quarantined
        // (for the undo toast), then stamp them.
        let targets: Vec<(String, String)> = sqlx::query_as(
            "SELECT id, person_id FROM voiceprints
             WHERE source_meeting_id = ? AND source_speaker_key = ?
               AND quarantined_at IS NULL AND person_id <> ?",
        )
        .bind(source_meeting_id)
        .bind(source_speaker_key)
        .bind(exclude_person_id)
        .fetch_all(pool)
        .await?;

        if targets.is_empty() {
            return Ok(Vec::new());
        }

        let now = Utc::now().to_rfc3339();
        sqlx::query(
            "UPDATE voiceprints SET quarantined_at = ?
             WHERE source_meeting_id = ? AND source_speaker_key = ?
               AND quarantined_at IS NULL AND person_id <> ?",
        )
        .bind(&now)
        .bind(source_meeting_id)
        .bind(source_speaker_key)
        .bind(exclude_person_id)
        .execute(pool)
        .await?;

        Ok(targets)
    }

    /// Wipe the entire gallery (ADR-0007 §6 "clear all voiceprints") — every stored
    /// voiceprint for every person. Leaves `people`/identity rows intact.
    pub async fn clear_all(pool: &SqlitePool) -> Result<u64, SqlxError> {
        let res = sqlx::query("DELETE FROM voiceprints").execute(pool).await?;
        Ok(res.rows_affected())
    }
}

/// Mean of a set of embedding blobs, re-L2-normalized; `None` if no usable blob.
/// Malformed or length-mismatched blobs are skipped (defensive — a corrupt sample must
/// not poison the centroid). The first usable blob fixes the expected dimension.
fn mean_centroid(blobs: &[Vec<u8>]) -> Option<Vec<f32>> {
    let mut acc: Vec<f32> = Vec::new();
    let mut count = 0u32;
    for blob in blobs {
        let v = match embedding_from_bytes(blob) {
            Ok(v) if !v.is_empty() => v,
            _ => continue,
        };
        if acc.is_empty() {
            acc = vec![0.0; v.len()];
        } else if acc.len() != v.len() {
            // A different-dimension blob can't be averaged in — skip it.
            continue;
        }
        for (a, x) in acc.iter_mut().zip(v.iter()) {
            *a += x;
        }
        count += 1;
    }
    if count == 0 {
        return None;
    }
    let inv = 1.0 / count as f32;
    for a in acc.iter_mut() {
        *a *= inv;
    }
    Some(l2_normalize(&acc))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diarization::embedding::cosine_similarity;

    const MODEL: &str = "3dspeaker_campplus_sv_en_voxceleb_16k";

    /// In-memory SQLite pool with the schema applied (mirrors the app pool's no-FK
    /// posture — `PRAGMA foreign_keys` is left OFF).
    async fn test_pool() -> SqlitePool {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        // Minimal people + voiceprints schema for these repo tests.
        sqlx::query(
            "CREATE TABLE people (
                id TEXT PRIMARY KEY, email TEXT, display_name TEXT NOT NULL,
                role TEXT, notes TEXT, voiceprint_opt_out INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL, updated_at TEXT NOT NULL)",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "CREATE TABLE voiceprints (
                id TEXT PRIMARY KEY, person_id TEXT NOT NULL, embedding BLOB NOT NULL,
                embedding_dim INTEGER NOT NULL, embedding_model TEXT NOT NULL,
                source_meeting_id TEXT, source_speaker_key TEXT, sample_quality REAL,
                quarantined_at TEXT, created_at TEXT NOT NULL)",
        )
        .execute(&pool)
        .await
        .unwrap();
        pool
    }

    #[tokio::test]
    async fn add_and_centroid_on_read() {
        let pool = test_pool().await;
        // Two samples that average to the x-axis direction.
        VoiceprintsRepository::add_sample(
            &pool,
            "p1",
            &[1.0, 1.0, 0.0],
            MODEL,
            None,
            None,
            Some(1.0),
        )
        .await
        .unwrap();
        VoiceprintsRepository::add_sample(
            &pool,
            "p1",
            &[1.0, -1.0, 0.0],
            MODEL,
            None,
            None,
            Some(1.0),
        )
        .await
        .unwrap();

        let c = VoiceprintsRepository::centroid_for_person(&pool, "p1", MODEL)
            .await
            .unwrap()
            .expect("centroid present");
        // Mean of the two normalized vectors points along +x.
        assert!(
            cosine_similarity(&c, &[1.0, 0.0, 0.0]) > 0.999,
            "centroid {c:?}"
        );
        assert_eq!(
            VoiceprintsRepository::count_for_person(&pool, "p1")
                .await
                .unwrap(),
            2
        );
    }

    #[tokio::test]
    async fn centroid_is_none_for_unknown_person_or_model() {
        let pool = test_pool().await;
        VoiceprintsRepository::add_sample(&pool, "p1", &[1.0, 0.0], MODEL, None, None, Some(1.0))
            .await
            .unwrap();
        assert!(
            VoiceprintsRepository::centroid_for_person(&pool, "nobody", MODEL)
                .await
                .unwrap()
                .is_none()
        );
        // Different model → no samples → no centroid.
        assert!(
            VoiceprintsRepository::centroid_for_person(&pool, "p1", "other_model")
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn prune_keeps_best_n_by_quality() {
        let pool = test_pool().await;
        // Insert MAX+5 samples with ascending quality; only the top MAX must survive.
        for i in 0..(MAX_SAMPLES_PER_PERSON + 5) {
            let q = i as f32; // higher i = higher quality
            VoiceprintsRepository::add_sample(&pool, "p1", &[1.0, 0.0], MODEL, None, None, Some(q))
                .await
                .unwrap();
        }
        assert_eq!(
            VoiceprintsRepository::count_for_person(&pool, "p1")
                .await
                .unwrap(),
            MAX_SAMPLES_PER_PERSON as i64
        );
        // The lowest qualities (0..5) must have been pruned; min surviving quality is 5.
        let min_q: Option<f64> = sqlx::query_scalar(
            "SELECT MIN(sample_quality) FROM voiceprints WHERE person_id = 'p1'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(min_q, Some(5.0));
    }

    /// specs/0039 WS3: the best-N prune must NEVER hard-delete a quarantined sample, even
    /// when the person is at the cap and the quarantined row is the weakest by quality —
    /// otherwise a retraction's quarantined sample could vanish before the user clicks Undo.
    /// Fill to the cap, quarantine the weakest (NULL-quality) row, then add another live
    /// sample to trigger a prune → the quarantined row must still exist and be restorable.
    #[tokio::test]
    async fn prune_never_evicts_a_quarantined_sample() {
        let pool = test_pool().await;
        // The victim: a NULL-quality sample — the PRIME eviction candidate (NULL sorts last).
        VoiceprintsRepository::add_sample(
            &pool,
            "p1",
            &[1.0, 0.0],
            MODEL,
            Some("m1"),
            Some("spk_weak"),
            None,
        )
        .await
        .unwrap();
        // Fill the rest of the cap with strong (high-quality) LIVE samples.
        for _ in 1..MAX_SAMPLES_PER_PERSON {
            VoiceprintsRepository::add_sample(
                &pool,
                "p1",
                &[1.0, 0.0],
                MODEL,
                Some("m1"),
                Some("spk_x"),
                Some(10.0),
            )
            .await
            .unwrap();
        }

        // Quarantine the weak sample (a retraction awaiting Undo).
        let victim = VoiceprintsRepository::list_for_person(&pool, "p1")
            .await
            .unwrap()
            .into_iter()
            .find(|s| s.source_speaker_key.as_deref() == Some("spk_weak"))
            .map(|s| s.id)
            .expect("weak sample present");
        assert!(VoiceprintsRepository::quarantine_sample(&pool, &victim)
            .await
            .unwrap());

        // Add another strong LIVE sample → LIVE count returns to the cap → prune runs.
        VoiceprintsRepository::add_sample(
            &pool,
            "p1",
            &[1.0, 0.0],
            MODEL,
            Some("m1"),
            Some("spk_x"),
            Some(10.0),
        )
        .await
        .unwrap();

        // The quarantined victim must still exist (the prune excluded it) and be restorable.
        let after = VoiceprintsRepository::list_for_person(&pool, "p1")
            .await
            .unwrap();
        assert!(
            after.iter().any(|s| s.id == victim && s.quarantined),
            "the quarantined sample must survive the prune (undo invariant)"
        );
        assert!(
            VoiceprintsRepository::restore_sample(&pool, &victim)
                .await
                .unwrap(),
            "the quarantined sample must still be restorable after a prune"
        );
    }

    #[tokio::test]
    async fn all_centroids_groups_by_person() {
        let pool = test_pool().await;
        VoiceprintsRepository::add_sample(&pool, "p1", &[1.0, 0.0], MODEL, None, None, Some(1.0))
            .await
            .unwrap();
        VoiceprintsRepository::add_sample(&pool, "p2", &[0.0, 1.0], MODEL, None, None, Some(1.0))
            .await
            .unwrap();
        let centroids = VoiceprintsRepository::all_centroids(&pool, MODEL)
            .await
            .unwrap();
        assert_eq!(centroids.len(), 2);
        let ids: Vec<&str> = centroids.iter().map(|(p, _, _)| p.as_str()).collect();
        assert!(ids.contains(&"p1") && ids.contains(&"p2"));
        assert!(
            centroids.iter().all(|(_, _, n)| *n == 1),
            "0044: count rides along"
        );
    }

    #[tokio::test]
    async fn delete_for_person_and_clear_all() {
        let pool = test_pool().await;
        VoiceprintsRepository::add_sample(&pool, "p1", &[1.0, 0.0], MODEL, None, None, Some(1.0))
            .await
            .unwrap();
        VoiceprintsRepository::add_sample(&pool, "p2", &[0.0, 1.0], MODEL, None, None, Some(1.0))
            .await
            .unwrap();
        assert_eq!(
            VoiceprintsRepository::delete_for_person(&pool, "p1")
                .await
                .unwrap(),
            1
        );
        assert_eq!(
            VoiceprintsRepository::count_for_person(&pool, "p1")
                .await
                .unwrap(),
            0
        );
        // p2 untouched.
        assert_eq!(
            VoiceprintsRepository::count_for_person(&pool, "p2")
                .await
                .unwrap(),
            1
        );
        let cleared = VoiceprintsRepository::clear_all(&pool).await.unwrap();
        assert_eq!(cleared, 1);
    }

    // ----- specs/0039 WS3: quarantine / list / retraction -----

    /// Insert a sample carrying a cluster back-link (source_meeting_id + source_speaker_key).
    async fn add_linked(pool: &SqlitePool, person_id: &str, emb: &[f32], meeting: &str, spk: &str) {
        VoiceprintsRepository::add_sample(
            pool,
            person_id,
            emb,
            MODEL,
            Some(meeting),
            Some(spk),
            None,
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn quarantine_excludes_from_centroid_but_keeps_row_and_restores() {
        let pool = test_pool().await;
        // The "good" sample points along +x; the "bad" one along +y. If the bad one is
        // excluded, the centroid must point along +x.
        add_linked(&pool, "p1", &[1.0, 0.0], "m1", "spk_good").await;
        add_linked(&pool, "p1", &[0.0, 1.0], "m1", "spk_bad").await;

        let bad_id = VoiceprintsRepository::list_for_person(&pool, "p1")
            .await
            .unwrap()
            .into_iter()
            .find(|s| s.source_speaker_key.as_deref() == Some("spk_bad"))
            .map(|s| s.id)
            .expect("bad sample present");

        assert!(VoiceprintsRepository::quarantine_sample(&pool, &bad_id)
            .await
            .unwrap());

        // Row survives (list still shows 2, one flagged); count counts all rows.
        let after = VoiceprintsRepository::list_for_person(&pool, "p1")
            .await
            .unwrap();
        assert_eq!(after.len(), 2, "quarantine keeps the row");
        assert_eq!(after.iter().filter(|s| s.quarantined).count(), 1);
        assert_eq!(
            VoiceprintsRepository::count_for_person(&pool, "p1")
                .await
                .unwrap(),
            2
        );

        // Centroid now excludes the +y sample → points along +x.
        let c = VoiceprintsRepository::centroid_for_person(&pool, "p1", MODEL)
            .await
            .unwrap()
            .expect("one live sample remains");
        assert!(
            cosine_similarity(&c, &[1.0, 0.0]) > 0.999,
            "centroid should exclude the quarantined +y sample: {c:?}"
        );

        // Restore → both live again → centroid returns to the 45° mean.
        assert!(VoiceprintsRepository::restore_sample(&pool, &bad_id)
            .await
            .unwrap());
        assert!(VoiceprintsRepository::list_for_person(&pool, "p1")
            .await
            .unwrap()
            .iter()
            .all(|s| !s.quarantined));
        let c2 = VoiceprintsRepository::centroid_for_person(&pool, "p1", MODEL)
            .await
            .unwrap()
            .unwrap();
        assert!(
            cosine_similarity(&c2, &[1.0, 1.0]) > 0.999,
            "restored centroid should be the 2-sample mean: {c2:?}"
        );
    }

    #[tokio::test]
    async fn delete_sample_removes_row() {
        let pool = test_pool().await;
        add_linked(&pool, "p1", &[1.0, 0.0], "m1", "spk_1").await;
        let id = VoiceprintsRepository::list_for_person(&pool, "p1")
            .await
            .unwrap()[0]
            .id
            .clone();
        assert!(VoiceprintsRepository::delete_sample(&pool, &id)
            .await
            .unwrap());
        assert_eq!(
            VoiceprintsRepository::count_for_person(&pool, "p1")
                .await
                .unwrap(),
            0
        );
        // Deleting again is a no-op (false).
        assert!(!VoiceprintsRepository::delete_sample(&pool, &id)
            .await
            .unwrap());
    }

    #[tokio::test]
    async fn quarantine_for_cluster_retracts_person_and_spares_owner() {
        let pool = test_pool().await;
        const OWNER: &str = "person-owner-self";
        // Person p1's sample from (m1, spk_1) — the polluted attribution to retract.
        add_linked(&pool, "p1", &[1.0, 0.0], "m1", "spk_1").await;
        // Owner's sample ALSO enrolled from (m1, spk_1) (e.g. "this attendee is me") — must
        // NOT be touched by a remote correction.
        add_linked(&pool, OWNER, &[0.0, 1.0], "m1", "spk_1").await;
        // A different meeting's sample for p1 — must survive (different source_meeting_id).
        add_linked(&pool, "p1", &[1.0, 1.0], "m2", "spk_1").await;

        let retracted = VoiceprintsRepository::quarantine_for_cluster(&pool, "m1", "spk_1", OWNER)
            .await
            .unwrap();
        // Exactly p1's m1/spk_1 sample is quarantined; owner excluded.
        assert_eq!(retracted.len(), 1);
        assert_eq!(retracted[0].1, "p1");

        // p1's m1 sample is now excluded from the centroid, but the m2 sample survives.
        let p1_live: Vec<_> = VoiceprintsRepository::list_for_person(&pool, "p1")
            .await
            .unwrap()
            .into_iter()
            .filter(|s| !s.quarantined)
            .collect();
        assert_eq!(p1_live.len(), 1);
        assert_eq!(p1_live[0].source_meeting_id.as_deref(), Some("m2"));

        // Owner gallery untouched: its (m1, spk_1) sample is still live.
        let owner_live = VoiceprintsRepository::list_for_person(&pool, OWNER)
            .await
            .unwrap();
        assert_eq!(owner_live.len(), 1);
        assert!(
            !owner_live[0].quarantined,
            "owner sample must not be retracted"
        );

        // Re-running the retraction quarantines nothing new (idempotent — no live matches).
        let again = VoiceprintsRepository::quarantine_for_cluster(&pool, "m1", "spk_1", OWNER)
            .await
            .unwrap();
        assert!(again.is_empty());
    }
}
