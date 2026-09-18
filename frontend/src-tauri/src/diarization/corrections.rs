//! Manual speaker-correction commands (specs/0019 WS2.3, specs/0039 WS2/WS3):
//! single-line and span reassignment, override clearing, manual speaker minting,
//! and the voiceprint retraction that a material span correction triggers.
//!
//! Split out of `diarization/commands.rs` under the specs/0042 file-size ratchet
//! (0044 follow-through). Command NAMES are unchanged — only the registration
//! path in `registry.rs` moved.

use serde::Serialize;
use sqlx::SqlitePool;
use tauri::{AppHandle, Emitter, Manager, Runtime};

use crate::database::repositories::people::PeopleRepository;
use crate::database::repositories::speaker::SpeakersRepository;
use crate::database::repositories::transcript_speaker_overrides::TranscriptSpeakerOverridesRepository;
use crate::diarization::commands::SpeakerDto;
use crate::diarization::speaker_maintenance::{ensure_local_speaker, prune_empty_speakers_inner};
use crate::diarization::LOCAL_SPEAKER_KEY;
use crate::state::AppState;

/// Reassign a SINGLE transcript line to another speaker (specs/0019 WS2.3, note 8) —
/// the per-segment "split" fix when diarization lumped one line under the wrong speaker.
/// Updates the live transcript AND records a sticky override (keyed by transcript id) so
/// the correction survives a later offline re-diarization (which otherwise rebuilds all
/// speaker keys from scratch). Errors if the line doesn't belong to the meeting.
/// Body of [`api_set_segment_speaker`], extracted so tests can drive it without an
/// `AppHandle`. See that command's docs. specs/0061 W4: when the target is the owner
/// (`LOCAL_SPEAKER_KEY`), ensures the `local`/"You" speaker row exists first — no mic-tagged
/// segment ever produced one for some meetings, which otherwise made "You" unreassignable —
/// and prunes any speaker the reassignment left empty afterward.
pub(crate) async fn set_segment_speaker_inner(
    pool: &SqlitePool,
    meeting_id: &str,
    transcript_id: &str,
    speaker_key: &str,
) -> Result<(), String> {
    if transcript_id.trim().is_empty() || speaker_key.trim().is_empty() {
        return Err("transcript_id and speaker_key cannot be empty".to_string());
    }

    if speaker_key == LOCAL_SPEAKER_KEY {
        ensure_local_speaker(pool, meeting_id)
            .await
            .map_err(|e| format!("Failed to ensure the owner speaker exists: {e}"))?;
    }

    // REVIEW(0039): single-line reassignment has no voiceprint retraction hook (only the SPAN
    // command `api_set_segment_speakers` does). A one-line correction is rarely material enough
    // to invalidate a cluster's voiceprint, so this is intentional for now — flag for the owner
    // rather than moving the hook here tonight.
    let applied =
        TranscriptSpeakerOverridesRepository::set(pool, meeting_id, transcript_id, speaker_key)
            .await
            .map_err(|e| format!("Failed to set segment speaker: {e}"))?;
    if !applied {
        return Err(format!(
            "No transcript '{transcript_id}' found for this meeting"
        ));
    }

    prune_empty_speakers_inner(pool, meeting_id)
        .await
        .map_err(|e| format!("Failed to prune empty speakers: {e}"))?;

    Ok(())
}

#[tauri::command]
pub async fn api_set_segment_speaker<R: Runtime>(
    app: AppHandle<R>,
    meeting_id: String,
    transcript_id: String,
    speaker_key: String,
) -> Result<(), String> {
    let state = app.state::<AppState>();
    let pool = state.db_manager.pool();
    set_segment_speaker_inner(pool, &meeting_id, &transcript_id, &speaker_key).await
}

/// Drop a line's manual speaker override (specs/0019 WS2.3) so it reverts to the
/// diarizer's assignment on the next pass. Idempotent (clearing an absent override is Ok).
#[tauri::command]
pub async fn api_clear_segment_speaker<R: Runtime>(
    app: AppHandle<R>,
    meeting_id: String,
    transcript_id: String,
) -> Result<(), String> {
    let state = app.state::<AppState>();
    let pool = state.db_manager.pool();

    TranscriptSpeakerOverridesRepository::clear(pool, &meeting_id, &transcript_id)
        .await
        .map(|_| ())
        .map_err(|e| format!("Failed to clear segment speaker: {e}"))
}

/// Reassign a SPAN of transcript lines to one target speaker in a single atomic write
/// (specs/0039 WS2) — the bulk form of [`api_set_segment_speaker`] for correcting a whole
/// mis-attributed stretch of a call at once. `transcript_ids` is the selected span; the
/// target `speaker_key` is an EXISTING speaker (`spk_N`/`local`) or a just-minted
/// `manual_<uuid>` from [`api_create_meeting_speaker`]. Writes the live
/// `transcripts.speaker` AND durable per-line overrides, so the correction survives a
/// later offline re-diarization (via `reapply`). Lines not belonging to the meeting are
/// skipped; returns the number of lines actually reassigned.
///
/// No event is emitted (matching the single-line path): the caller re-fetches speakers +
/// transcripts after this resolves, per the specs/0039 Design ("No new events").
/// Body of [`api_set_segment_speakers`], extracted so tests can drive it without an
/// `AppHandle`. See that command's docs. Returns `(reassigned count, retraction target
/// keys)` — the caller (the real command) uses the latter to emit `voiceprint-retracted`,
/// which needs an `AppHandle` this function doesn't have. specs/0061 W4: when the target is
/// the owner, ensures the `local`/"You" row exists before applying, and prunes any speaker
/// the reassignment left empty afterward.
pub(crate) async fn set_segment_speakers_inner(
    pool: &SqlitePool,
    meeting_id: &str,
    transcript_ids: Vec<String>,
    speaker_key: &str,
) -> Result<(u64, Vec<String>), String> {
    if speaker_key.trim().is_empty() {
        return Err("speaker_key cannot be empty".to_string());
    }
    if transcript_ids.is_empty() {
        return Err("Select at least one line to reassign".to_string());
    }

    if speaker_key == LOCAL_SPEAKER_KEY {
        ensure_local_speaker(pool, meeting_id)
            .await
            .map_err(|e| format!("Failed to ensure the owner speaker exists: {e}"))?;
    }

    // specs/0039 WS3 retraction hook (task 8). Capture the "corrected-away" cluster keys
    // and their material fraction BEFORE `set_many` overwrites `transcripts.speaker` — once
    // the span is reassigned we can no longer tell which key each line used to hold. This is
    // pure reads and fully best-effort: any error just yields an empty target set so the
    // reassignment below is never blocked.
    let retraction_targets =
        capture_retraction_targets(pool, meeting_id, &transcript_ids, speaker_key).await;

    let applied = TranscriptSpeakerOverridesRepository::set_many(
        pool,
        meeting_id,
        &transcript_ids,
        speaker_key,
    )
    .await
    .map_err(|e| format!("Failed to reassign the selected lines: {e}"))?;
    if applied == 0 {
        return Err("None of the selected lines belong to this meeting".to_string());
    }

    prune_empty_speakers_inner(pool, meeting_id)
        .await
        .map_err(|e| format!("Failed to prune empty speakers: {e}"))?;

    Ok((applied, retraction_targets))
}

#[tauri::command]
pub async fn api_set_segment_speakers<R: Runtime>(
    app: AppHandle<R>,
    meeting_id: String,
    transcript_ids: Vec<String>,
    speaker_key: String,
) -> Result<u64, String> {
    let state = app.state::<AppState>();
    let pool = state.db_manager.pool();

    let (applied, retraction_targets) =
        set_segment_speakers_inner(pool, &meeting_id, transcript_ids, &speaker_key).await?;

    // Retract (quarantine) the voiceprint samples the materially-corrected-away clusters
    // contributed, and emit `voiceprint-retracted` per affected person so the UI can offer
    // an undo. Best-effort / log-and-continue: a quarantine failure must NEVER fail the
    // (already-committed) reassignment. The owner ("You") gallery is never touched.
    if !retraction_targets.is_empty() {
        retract_and_notify(&app, pool, &meeting_id, &retraction_targets).await;
    }

    Ok(applied)
}

/// Fraction threshold for the WS3 retraction (specs/0039 owner decision), applied on BOTH
/// axes so a retraction is conservative + targeted:
/// - **Span-dominance:** the corrected-away key must hold at least this fraction of the
///   SELECTED span — i.e. it's what the user was actually fixing, not a small bystander
///   cluster incidentally swept into a large span.
/// - **Material-fraction-of-cluster:** the reassignment must invalidate at least this
///   fraction of the key's own meeting turns, so its enrolled voiceprint is genuinely
///   suspect. Below it the cluster is still mostly right and we leave its gallery alone.
///
/// Shared with the enroll gate's contested test ([`MATERIAL_CONTEST_FRACTION`]) as the one
/// "materially contested" constant.
///
/// [`MATERIAL_CONTEST_FRACTION`]: crate::database::repositories::transcript_speaker_overrides::MATERIAL_CONTEST_FRACTION
const MATERIAL_RETRACTION_FRACTION: f64 =
    crate::database::repositories::transcript_speaker_overrides::MATERIAL_CONTEST_FRACTION;

/// Tauri event emitted when a WS2 span correction quarantines voiceprint samples (specs/0039
/// WS3). Payload: `{ meetingId, personId, personName, quarantinedSampleIds }` — the WS3
/// frontend listens for this to show an "undo" toast that calls `api_restore_voiceprint_sample`
/// on each id. One event is emitted per affected person.
pub const EVENT_VOICEPRINT_RETRACTED: &str = "voiceprint-retracted";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct VoiceprintRetractedPayload {
    meeting_id: String,
    person_id: String,
    person_name: String,
    quarantined_sample_ids: Vec<String>,
}

/// The durable Person a meeting's speaker is linked to, if any (specs/0039 WS3). Used by the
/// retraction's same-person drift-merge guard. Best-effort: any DB error or missing link → None.
async fn person_id_for_speaker(
    pool: &sqlx::SqlitePool,
    meeting_id: &str,
    speaker_key: &str,
) -> Option<String> {
    sqlx::query_scalar::<_, Option<String>>(
        "SELECT person_id FROM speakers WHERE meeting_id = ? AND speaker_key = ?",
    )
    .bind(meeting_id)
    .bind(speaker_key)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten()
    .flatten()
}

/// Read the corrected-away cluster keys that should have their voiceprint samples retracted
/// (specs/0039 WS3). Must run BEFORE the span is overwritten. Conservative + targeted — a key
/// qualifies only when ALL of:
///  1. **Span-dominance** — it holds ≥ [`MATERIAL_RETRACTION_FRACTION`] of the SELECTED span,
///     i.e. it's what the user was actually correcting (a small bystander cluster incidentally
///     covered by a large span fails this and is spared).
///  2. **Material-fraction-of-cluster** — the span invalidates ≥ that fraction of the key's own
///     meeting turns, so its enrolled voiceprint is genuinely suspect.
///  3. **Not a same-person drift-merge** — the reassignment target resolves to a DIFFERENT
///     `person_id` than the corrected-away key. Reassigning spk_1→spk_2 where both are person A
///     must never discard person A's own samples.
///
/// Returns the qualifying keys; empty (and swallows any DB error) so the caller's reassignment
/// is never blocked by this best-effort read.
async fn capture_retraction_targets(
    pool: &sqlx::SqlitePool,
    meeting_id: &str,
    transcript_ids: &[String],
    new_speaker_key: &str,
) -> Vec<String> {
    if transcript_ids.is_empty() {
        return Vec::new();
    }

    // Per-key count of span lines currently held by each corrected-away key (anything in the
    // span that isn't already the new target). Dynamic `IN (?, ?, …)` over the selected ids.
    let placeholders = std::iter::repeat_n("?", transcript_ids.len())
        .collect::<Vec<_>>()
        .join(", ");
    let span_sql = format!(
        "SELECT speaker, COUNT(*) FROM transcripts
         WHERE meeting_id = ? AND speaker IS NOT NULL AND speaker <> ?
           AND id IN ({placeholders})
         GROUP BY speaker"
    );
    let mut span_q = sqlx::query_as::<_, (String, i64)>(&span_sql)
        .bind(meeting_id)
        .bind(new_speaker_key);
    for id in transcript_ids {
        span_q = span_q.bind(id);
    }
    let span_counts: Vec<(String, i64)> = match span_q.fetch_all(pool).await {
        Ok(rows) => rows,
        Err(e) => {
            log::warn!(
                "voiceprint-retraction: span capture failed for {meeting_id} (skipping): {e}"
            );
            return Vec::new();
        }
    };
    if span_counts.is_empty() {
        return Vec::new();
    }

    // One GROUP BY for every speaker's total turns in the meeting (replaces the per-key
    // COUNT N+1) — read each key's material denominator from this map.
    let total_counts: std::collections::HashMap<String, i64> =
        match sqlx::query_as::<_, (String, i64)>(
            "SELECT speaker, COUNT(*) FROM transcripts
         WHERE meeting_id = ? AND speaker IS NOT NULL
         GROUP BY speaker",
        )
        .bind(meeting_id)
        .fetch_all(pool)
        .await
        {
            Ok(rows) => rows.into_iter().collect(),
            Err(e) => {
                log::warn!(
                    "voiceprint-retraction: total counts failed for {meeting_id} (skipping): {e}"
                );
                return Vec::new();
            }
        };

    // The reassignment target's Person — the same-person drift-merge guard compares against it.
    let target_person = person_id_for_speaker(pool, meeting_id, new_speaker_key).await;
    let span_len = transcript_ids.len() as f64;

    let mut material = Vec::new();
    for (key, span_count) in span_counts {
        // Never retract via the mixed "unknown" bucket — it carries no voiceprint anyway.
        if key == crate::diarization::UNKNOWN_SPEAKER_KEY {
            continue;
        }
        let span_count = span_count as f64;

        // (1) Span-dominance: was this key the actual target of the correction? A 2-line
        // bystander in a 40-line span (0.05) fails and is left alone.
        if span_count / span_len < MATERIAL_RETRACTION_FRACTION {
            continue;
        }

        // (2) Material fraction of the key's own cluster.
        let total = total_counts.get(&key).copied().unwrap_or(0);
        if !(total > 0 && span_count / (total as f64) >= MATERIAL_RETRACTION_FRACTION) {
            continue;
        }

        // (3) Same-person drift-merge guard: skip when the target is the same Person as the
        // corrected-away key (never discard a person's own samples on a self-merge).
        if let Some(target) = target_person.as_deref() {
            if let Some(key_person) = person_id_for_speaker(pool, meeting_id, &key).await {
                if key_person == target {
                    log::info!(
                        "voiceprint-retraction: skipping same-person drift-merge of {key} in {meeting_id}"
                    );
                    continue;
                }
            }
        }

        material.push(key);
    }
    material
}

/// Quarantine the voiceprint samples the given corrected-away clusters contributed for this
/// meeting and emit `voiceprint-retracted` per affected person (specs/0039 WS3). Best-effort:
/// logs and continues on any failure. The owner ("You") gallery is excluded — a remote
/// correction never touches the owner's self-enrolled voice.
async fn retract_and_notify<R: Runtime>(
    app: &AppHandle<R>,
    pool: &sqlx::SqlitePool,
    meeting_id: &str,
    material_keys: &[String],
) {
    use crate::database::repositories::voiceprints::VoiceprintsRepository;
    use std::collections::BTreeMap;

    // person_id -> quarantined sample ids, aggregated across all corrected-away keys.
    let mut by_person: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for key in material_keys {
        match VoiceprintsRepository::quarantine_for_cluster(
            pool,
            meeting_id,
            key,
            crate::people::enroll::OWNER_PERSON_ID,
        )
        .await
        {
            Ok(rows) => {
                for (sample_id, person_id) in rows {
                    by_person.entry(person_id).or_default().push(sample_id);
                }
            }
            Err(e) => log::warn!(
                "voiceprint-retraction: quarantine of cluster {key} in {meeting_id} failed (continuing): {e}"
            ),
        }
    }

    for (person_id, sample_ids) in by_person {
        let person_name = PeopleRepository::get(pool, &person_id)
            .await
            .ok()
            .flatten()
            .map(|p| p.display_name)
            .unwrap_or_else(|| "this person".to_string());
        log::info!(
            "voiceprint-retraction: quarantined {} sample(s) for person {person_id} after span correction in {meeting_id}",
            sample_ids.len()
        );
        let _ = app.emit(
            EVENT_VOICEPRINT_RETRACTED,
            VoiceprintRetractedPayload {
                meeting_id: meeting_id.to_string(),
                person_id,
                person_name,
                quarantined_sample_ids: sample_ids,
            },
        );
    }
}

/// Mint a brand-new speaker for a meeting so a mis-clustered span can be moved to an
/// identity the diarizer never produced (specs/0039 WS2 "New speaker…"). Creates a
/// `manual_<uuid>` `speakers` row with the given display name and a **NULL embedding**
/// (there is no audio to derive a voiceprint from), and returns it as a [`SpeakerDto`]
/// the caller can drop straight into the speaker legend. Pair with
/// [`api_set_segment_speakers`] to reassign the span onto the returned `speakerKey`.
///
/// Because the row has no voiceprint it is inert for cross-meeting matching and voiceprint
/// enrollment; it persists only through the per-line override table (see
/// [`SpeakersRepository::create_manual`](crate::database::repositories::speaker::SpeakersRepository::create_manual)).
#[tauri::command]
pub async fn api_create_meeting_speaker<R: Runtime>(
    app: AppHandle<R>,
    meeting_id: String,
    display_name: String,
) -> Result<SpeakerDto, String> {
    let display_name = display_name.trim();
    if meeting_id.trim().is_empty() {
        return Err("A meeting is required".to_string());
    }
    if display_name.is_empty() {
        return Err("A name is required to create a speaker".to_string());
    }
    let state = app.state::<AppState>();
    let pool = state.db_manager.pool();

    let speaker_key = SpeakersRepository::create_manual(pool, &meeting_id, display_name)
        .await
        .map_err(|e| format!("Failed to create speaker: {e}"))?;

    Ok(SpeakerDto {
        speaker_key,
        display_name: display_name.to_string(),
        is_local: false,
        email: None,
        person_id: None,
    })
}

/// Shared test fixtures for the correction commands AND
/// [`crate::diarization::speaker_maintenance`] (specs/0061 W4) — `pub(crate)` so both
/// modules' `#[cfg(test)]` code can build on the same in-memory-DB helpers.
#[cfg(test)]
pub(crate) mod test_support {
    use crate::database::repositories::speaker::SpeakersRepository;
    use crate::diarization::LOCAL_SPEAKER_KEY;
    use sqlx::SqlitePool;
    use uuid::Uuid;

    /// In-memory pool through the app's real migration set (matches the repo tests).
    /// `foreign_keys` is ON by sqlx default, so transcripts need their meeting and a
    /// speaker's `person_id` must reference a real people row.
    pub(crate) async fn pool_with_schema() -> SqlitePool {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        pool
    }

    /// Insert a meeting with a fresh, unique id and return it.
    pub(crate) async fn insert_meeting(pool: &SqlitePool) -> String {
        let id = format!("meeting-{}", Uuid::new_v4());
        let now = chrono::Utc::now().to_rfc3339();
        sqlx::query("INSERT INTO meetings (id, title, created_at, updated_at) VALUES (?, ?, ?, ?)")
            .bind(&id)
            .bind("t")
            .bind(&now)
            .bind(&now)
            .execute(pool)
            .await
            .unwrap();
        id
    }

    async fn insert_lines_inner(
        pool: &SqlitePool,
        meeting: &str,
        start: usize,
        n: usize,
        speaker: Option<&str>,
    ) -> Vec<String> {
        let now = chrono::Utc::now().to_rfc3339();
        let mut ids = Vec::new();
        for i in start..start + n {
            let id = format!("{meeting}-t{i}");
            sqlx::query(
                "INSERT INTO transcripts (id, meeting_id, transcript, timestamp, audio_start_time, speaker)
                 VALUES (?, ?, ?, ?, ?, ?)",
            )
            .bind(&id)
            .bind(meeting)
            .bind("hi")
            .bind(&now)
            .bind(i as f64)
            .bind(speaker)
            .execute(pool)
            .await
            .unwrap();
            ids.push(id);
        }
        ids
    }

    /// Insert `n` unassigned (`speaker IS NULL`) transcript lines for `meeting`, each with a
    /// distinct ascending `audio_start_time` (so ordering tests have something real to
    /// override); returns their ids.
    pub(crate) async fn insert_lines(pool: &SqlitePool, meeting: &str, n: usize) -> Vec<String> {
        insert_lines_inner(pool, meeting, 0, n, None).await
    }

    /// Insert `n` transcript lines `{meeting}-t{start}..` for `meeting` under `speaker`;
    /// returns their ids.
    pub(crate) async fn insert_lines_with_speaker(
        pool: &SqlitePool,
        meeting: &str,
        start: usize,
        n: usize,
        speaker: &str,
    ) -> Vec<String> {
        insert_lines_inner(pool, meeting, start, n, Some(speaker)).await
    }

    /// A speaker row with the given display name (`is_local` inferred from the key).
    pub(crate) async fn insert_speaker(
        pool: &SqlitePool,
        meeting: &str,
        key: &str,
        display_name: &str,
    ) {
        SpeakersRepository::upsert(
            pool,
            meeting,
            key,
            display_name,
            key == LOCAL_SPEAKER_KEY,
            None,
            None,
            None,
        )
        .await
        .unwrap();
    }

    /// A speaker row (display name defaults to the key) linked to a durable person.
    pub(crate) async fn insert_speaker_with_person(
        pool: &SqlitePool,
        meeting: &str,
        key: &str,
        person_id: &str,
    ) {
        insert_speaker(pool, meeting, key, key).await;
        sqlx::query("UPDATE speakers SET person_id = ? WHERE meeting_id = ? AND speaker_key = ?")
            .bind(person_id)
            .bind(meeting)
            .bind(key)
            .execute(pool)
            .await
            .unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::*;
    use super::*;

    /// specs/0039 WS3: a small bystander cluster incidentally swept into a large span must
    /// NOT be retracted — only the span-dominant corrected-away speaker is.
    #[tokio::test]
    async fn bystander_in_large_span_is_not_retracted() {
        let pool = pool_with_schema().await;
        let m = insert_meeting(&pool).await;
        // 38 dominant lines + a 2-line innocent bystander = a 40-line span reassigned to spk_2.
        let mut span = insert_lines_with_speaker(&pool, &m, 0, 38, "spk_1").await;
        span.extend(insert_lines_with_speaker(&pool, &m, 38, 2, "spk_bystander").await);

        let material = capture_retraction_targets(&pool, &m, &span, "spk_2").await;
        assert!(
            material.contains(&"spk_1".to_string()),
            "the span-dominant speaker is retracted"
        );
        assert!(
            !material.contains(&"spk_bystander".to_string()),
            "a 2-line bystander in a 40-line span must be spared"
        );
    }

    /// specs/0039 WS3: reassigning spk_1→spk_2 where BOTH resolve to the same person (a
    /// drift-split of one voice) must NOT discard that person's own samples.
    #[tokio::test]
    async fn same_person_drift_merge_is_not_retracted() {
        let pool = pool_with_schema().await;
        let m = insert_meeting(&pool).await;
        let person = PeopleRepository::create(&pool, "Ana", None, None, None)
            .await
            .unwrap();
        insert_speaker_with_person(&pool, &m, "spk_1", &person.id).await;
        insert_speaker_with_person(&pool, &m, "spk_2", &person.id).await;
        let span = insert_lines_with_speaker(&pool, &m, 0, 10, "spk_1").await;

        let material = capture_retraction_targets(&pool, &m, &span, "spk_2").await;
        assert!(
            material.is_empty(),
            "a same-person drift-merge must not retract the person's own voice"
        );
    }

    /// specs/0039 WS3: a genuine full-speaker correction (every spk_1 line moved to a
    /// DIFFERENT person's spk_2) IS retracted.
    #[tokio::test]
    async fn genuine_full_speaker_correction_is_retracted() {
        let pool = pool_with_schema().await;
        let m = insert_meeting(&pool).await;
        let a = PeopleRepository::create(&pool, "A", None, None, None)
            .await
            .unwrap();
        let b = PeopleRepository::create(&pool, "B", None, None, None)
            .await
            .unwrap();
        insert_speaker_with_person(&pool, &m, "spk_1", &a.id).await;
        insert_speaker_with_person(&pool, &m, "spk_2", &b.id).await;
        let span = insert_lines_with_speaker(&pool, &m, 0, 10, "spk_1").await;

        let material = capture_retraction_targets(&pool, &m, &span, "spk_2").await;
        assert_eq!(material, vec!["spk_1".to_string()]);
    }
}
