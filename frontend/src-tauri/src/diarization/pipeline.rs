//! Offline diarization orchestration (specs/0010, ADR-0005, P1-B2).
//!
//! The public entry point, `diarize_meeting(app, meeting_id)`, lives in
//! [`crate::diarization::launch`] (run-slot guard, spawn, queue reporting, terminal
//! event — specs/0063 W3). This module holds the pass itself, run by `launch` on a
//! background task and emitting progress/complete/error events:
//!
//! 1. Resolve the meeting's recording folder (`meetings.folder_path`) → its
//!    `system.wav` (`audio::system_channel_wav`). Errors clearly if absent —
//!    meetings recorded before P1-B1 have no per-channel audio.
//! 2. Ensure the two ONNX models are present (download on demand), or error.
//! 3. Decode `system.wav` to 16 kHz mono f32.
//! 4. Run [`SherpaDiarizer`] (auto speaker count) → recording-relative speaker turns
//!    for the **remote** participants (`spk_0`,`spk_1`,…).
//! 5. Load the meeting's transcript segments (id + recording-relative times +
//!    capture-channel tag, specs/0029 WS3.4).
//! 6. Attribute each segment channel-aware (see
//!    [`align_turns_to_segments`](crate::diarization::align::align_turns_to_segments)):
//!    a `microphone`-tagged segment is the **local user** (`local`/"You")
//!    unconditionally — a system turn can never overwrite it; a `system`-tagged
//!    segment takes the max-overlap system turn (no overlap → the "unknown"
//!    bucket); a `mixed`/untagged (legacy NULL) segment takes the max-overlap
//!    turn, else the nearest turn within 1.0s, else the "unknown" bucket —
//!    never "You" (specs/0043 W1.3).
//! 7. Persist: clear prior keys (idempotent re-run) → UPDATE `transcripts.speaker`
//!    per segment → UPSERT `speakers` rows (`local`→"You" is_local=1, `spk_N`→
//!    "Speaker {N+1}").
//!
//! The heavy steps (model download, decode, `compute`) are blocking and run on
//! `spawn_blocking`; DB work is async on the pool.

use anyhow::{anyhow, Context, Result};
use std::path::PathBuf;
use tauri::{AppHandle, Emitter, Manager, Runtime};

use crate::audio::channel_writer::{system_channel_path, system_channel_wav};
use crate::database::repositories::speaker::{SpeakerIdentitySnapshot, SpeakersRepository};
use crate::database::repositories::transcript_speaker_overrides::TranscriptSpeakerOverridesRepository;
use crate::diarization::align::{
    align_turns_to_segments, AlignMode, AlignableSegment, Channel, LOCAL_SPEAKER_KEY,
};
use crate::diarization::models;
use crate::diarization::segments::load_segments;
use crate::diarization::split::{split_straddling_rows, SplitMode};
use crate::diarization::{Diarizer, SherpaDiarizer};
use crate::state::AppState;

/// Sample rate the diarizer requires (system.wav is already 16 kHz mono).
const DIARIZATION_SAMPLE_RATE: u32 = 16_000;

/// Event names (Rust → frontend). P1-C consumes these.
pub const EVENT_PROGRESS: &str = "diarization-progress";
pub const EVENT_COMPLETE: &str = "diarization-complete";
pub const EVENT_ERROR: &str = "diarization-error";
/// "This meeting's speaker rows changed; refetch the transcript."
///
/// Owner feedback 2026-09-21: "Is it possible to update the transcript with speakers as
/// they get identified, especially if they match a voice print, instead of waiting to the
/// very end?" The sherpa pass itself cannot help — its cluster ids only exist once the whole
/// file is clustered — but everything AFTER it used to land in one silent lump: align,
/// persist, match against the voiceprint gallery, auto-label, then one `diarization-complete`
/// that finally told the UI to look. The cluster labels were durably in SQLite two stages
/// before anything said so.
///
/// So this fires twice: once the clusters are persisted (generic "Speaker 1/2/3" appear) and
/// again once the gallery matches are applied (real names appear). It is advisory — dropping
/// it costs nothing, because `EVENT_COMPLETE` still arrives.
pub const EVENT_SPEAKERS_UPDATED: &str = "diarization-speakers-updated";

/// Stages reported after the sherpa pass returns. Before 2026-09-21 there were none: the
/// button sat on "Identifying… 100%" through alignment, persistence, the cross-meeting
/// matcher and auto-labelling, which reads as hung.
const STAGE_ATTRIBUTING: &str = "attributing";
const STAGE_MATCHING: &str = "matching known voices";
const STAGE_LABELLING: &str = "labelling";

// ---------------------------------------------------------------------------
// Per-meeting in-flight run registry (WS3.1, specs/0029).
//
// `api_diarize_meeting` had no "already running" guard, and the frontend's
// in-progress state was component-local — so a page remount re-enabled the button
// and a second concurrent run interleaved its 0→100 progress with the first (the
// oscillating percentage), then clobbered the first run's persist. The registry
// makes the backend the source of truth: at most one live run per meeting, and a
// queryable status (`api_diarization_status`) the UI rehydrates from on mount.
// ---------------------------------------------------------------------------

/// Queryable status of a meeting's diarization run. Serialized camelCase for the
/// `api_diarization_status` command. After a run ends the last terminal status
/// (`running: false`, stage `"complete"`/`"error"`) stays queryable for the
/// lifetime of the process (one small entry per diarized meeting).
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiarizationRunStatus {
    pub running: bool,
    pub stage: String,
    pub progress_pct: u8,
}

/// The registry itself. A plain module-level map (mirrors `lib.rs`'s
/// `LANGUAGE_PREFERENCE` pattern) rather than Tauri managed state so the
/// blocking-thread progress callbacks can update it without an `AppHandle`.
static RUN_REGISTRY: std::sync::LazyLock<
    std::sync::Mutex<std::collections::HashMap<String, DiarizationRunStatus>>,
> = std::sync::LazyLock::new(|| std::sync::Mutex::new(std::collections::HashMap::new()));

/// Lock the registry, recovering from poisoning (status is advisory — a panicked
/// writer must never wedge every future run).
fn registry_lock(
) -> std::sync::MutexGuard<'static, std::collections::HashMap<String, DiarizationRunStatus>> {
    RUN_REGISTRY
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Atomically claim the run slot for a meeting. Returns `false` (and changes
/// nothing) when a run is already live for it.
pub(super) fn registry_try_begin(meeting_id: &str) -> bool {
    let mut map = registry_lock();
    if map.get(meeting_id).is_some_and(|s| s.running) {
        return false;
    }
    map.insert(
        meeting_id.to_string(),
        DiarizationRunStatus {
            running: true,
            stage: "starting".to_string(),
            progress_pct: 0,
        },
    );
    true
}

/// Update the live run's stage/percentage. A stage change resets the percentage
/// (each stage owns its own 0→100 ramp); no-op once the run is terminal.
fn registry_update(meeting_id: &str, stage: &str, pct: Option<u8>) {
    let mut map = registry_lock();
    if let Some(s) = map.get_mut(meeting_id) {
        if !s.running {
            return;
        }
        if s.stage != stage {
            s.stage = stage.to_string();
            s.progress_pct = 0;
        }
        if let Some(p) = pct {
            s.progress_pct = p.min(100);
        }
    }
}

/// Mark the run terminal (`"complete"` / `"error"`), releasing the per-meeting
/// slot so a new run can start. Called BEFORE the matching event emit so a status
/// query racing the event never reads `running: true` after completion.
pub(super) fn registry_finish(meeting_id: &str, stage: &str, pct: u8) {
    let mut map = registry_lock();
    map.insert(
        meeting_id.to_string(),
        DiarizationRunStatus {
            running: false,
            stage: stage.to_string(),
            progress_pct: pct,
        },
    );
}

/// The current (or last terminal) diarization status for a meeting, if any run
/// happened this session. Consumed by `api_diarization_status`.
pub fn run_status(meeting_id: &str) -> Option<DiarizationRunStatus> {
    registry_lock().get(meeting_id).cloned()
}

/// Resolve a meeting's `system.wav` from its persisted recording folder.
///
/// Happy path: `meetings.folder_path` is set and its `system.wav` exists → use it.
///
/// Fallback: `folder_path` is NULL (the frontend save can race the folder write, see
/// the module-level note) or points somewhere without a `system.wav`. We then scan
/// the recordings root and match a folder by the meeting's `created_at`/title
/// ([`folder_match::best_folder_match`]). On a confident match we use it and
/// opportunistically backfill `folder_path` so subsequent reads (and the "open
/// meeting folder" feature) work without re-scanning. Errors are user-actionable.
async fn resolve_system_wav<R: Runtime>(app: &AppHandle<R>, meeting_id: &str) -> Result<PathBuf> {
    let state = app.state::<AppState>();
    let pool = state.db_manager.pool();

    let meta = crate::database::repositories::meeting::MeetingsRepository::get_meeting_metadata(
        pool, meeting_id,
    )
    .await
    .with_context(|| format!("look up meeting {meeting_id}"))?
    .ok_or_else(|| anyhow!("meeting {meeting_id} not found"))?;

    // Happy path: stored folder with a system channel (`system.wav`, or `.opus` once kept
    // audio is compressed, specs/0072).
    if let Some(folder) = meta.folder_path.as_deref() {
        if let Some(wav) = system_channel_path(std::path::Path::new(folder)) {
            return Ok(wav);
        }
        log::warn!(
            "meeting {meeting_id} folder_path is set ({folder}) but has no system channel; \
             falling back to a recordings-root scan"
        );
    }

    // Fallback: locate the recording folder under any known recordings root by name/time.
    let resolved = super::folder_locate::locate_recording_folder(&meta).with_context(|| {
        format!("locate recording folder for meeting {meeting_id} (folder_path was unusable)")
    })?;
    let wav = system_channel_path(&resolved).unwrap_or_else(|| system_channel_wav(&resolved));

    // Opportunistic backfill so we don't re-scan next time. Best-effort: log + continue.
    let folder_str = resolved.to_string_lossy().to_string();
    match crate::database::repositories::meeting::MeetingsRepository::update_folder_path(
        pool,
        meeting_id,
        &folder_str,
    )
    .await
    {
        Ok(true) => log::info!("backfilled folder_path for meeting {meeting_id} -> {folder_str}"),
        Ok(false) => log::warn!("folder_path backfill for meeting {meeting_id} updated no rows"),
        Err(e) => log::warn!("folder_path backfill for meeting {meeting_id} failed: {e}"),
    }

    Ok(wav)
}

/// Display name for a speaker key. `local` → "You"; `spk_N` → "Speaker {N+1}";
/// `unknown` (the WS3.3 overflow bucket) → "Unknown speaker"; anything else (e.g.
/// a fallback key) is passed through best-effort.
pub fn display_name_for_key(key: &str) -> String {
    if key == LOCAL_SPEAKER_KEY {
        return "You".to_string();
    }
    if key == crate::diarization::UNKNOWN_SPEAKER_KEY {
        return "Unknown speaker".to_string();
    }
    if let Some(n) = key.strip_prefix("spk_") {
        if let Ok(idx) = n.parse::<u32>() {
            return format!("Speaker {}", idx + 1);
        }
    }
    key.to_string()
}

/// Persist alignment results in one transaction: clear prior keys, set each
/// segment's `speaker`, and upsert the `speakers` rows. Idempotent across re-runs.
///
/// `embeddings` is the `spk_N → L2-normalized vector` map from
/// `diarize_with_embeddings` (specs/0016 1a). For each REMOTE speaker that has an
/// embedding, we persist the serialized bytes + dim + [`EMBEDDING_MODEL_ID`] so the
/// cross-meeting matcher has prior art to compare against. The `local`/"You" key is
/// never voiceprinted here (ADR-0007 §3 self-enroll is a 1c behavior), so it keeps a
/// NULL embedding. A re-run refreshes embeddings on `ON CONFLICT` (widened `upsert`).
async fn persist(
    pool: &sqlx::SqlitePool,
    meeting_id: &str,
    assignments: &[(String, String)],
    embeddings: &std::collections::HashMap<String, Vec<f32>>,
) -> Result<usize> {
    use crate::diarization::embedding::{embedding_to_bytes, EMBEDDING_MODEL_ID};
    use sqlx::Acquire;
    use std::collections::BTreeSet;

    // WS3.2 (specs/0029) — manual speaker RENAMES must survive this re-run too. The
    // clear below deletes every `speakers` row, so snapshot the ones the user
    // actually touched (display_name differs from the generated default for their
    // key) BEFORE clearing; they are re-applied after the fresh upsert. Queried
    // before acquiring the dedicated connection below so a small pool never
    // deadlocks on a second checkout.
    let renamed_snapshots: Vec<SpeakerIdentitySnapshot> =
        SpeakersRepository::get_identity_snapshots(pool, meeting_id)
            .await
            .context("snapshot prior speaker identities")?
            .into_iter()
            .filter(|s| s.display_name != display_name_for_key(&s.speaker_key))
            .collect();

    let mut conn = pool.acquire().await.context("acquire db connection")?;

    // specs/0019 WS2.3 — manual per-segment corrections are sticky: they must survive
    // this re-diarization. Pull the meeting's override keys so we also materialize a
    // `speakers` row for any key the fresh clustering didn't produce (a corrected line
    // pointing at it would otherwise resolve to no name).
    let override_keys =
        TranscriptSpeakerOverridesRepository::override_keys_for_meeting(&mut conn, meeting_id)
            .await
            .context("load per-segment override keys")?;

    // Distinct speaker keys to materialize: those assigned by clustering + any referenced
    // by a manual override.
    let mut keys: BTreeSet<String> = assignments.iter().map(|(_, k)| k.clone()).collect();
    keys.extend(override_keys.iter().cloned());

    let mut tx = conn
        .begin()
        .await
        .context("begin diarization transaction")?;

    // Clear prior keys + speaker rows so a re-run never leaves stale labels.
    SpeakersRepository::clear_meeting_speakers(&mut tx, meeting_id)
        .await
        .context("clear prior speakers")?;

    SpeakersRepository::set_segment_speakers(&mut tx, assignments)
        .await
        .context("write per-segment speaker keys")?;

    // Re-apply manual per-segment corrections ON TOP of the fresh clustering so they win
    // (specs/0019 WS2.3). Keyed by the stable transcript id, so they re-attach exactly.
    TranscriptSpeakerOverridesRepository::reapply(&mut tx, meeting_id)
        .await
        .context("re-apply per-segment speaker overrides")?;

    tx.commit().await.context("commit segment speaker keys")?;
    // Return the dedicated connection: everything below runs on the pool, and a
    // small pool (e.g. the 1-connection test pool) must not deadlock on checkout.
    drop(conn);

    // Upsert speakers on the pool (upsert is its own statement; outside the tx is
    // fine and keeps the repo API simple).
    for key in &keys {
        let is_local = key.as_str() == LOCAL_SPEAKER_KEY;

        // Voiceprint only REMOTE speakers (`local` stays NULL — ADR-0007 §3).
        let bytes = if is_local {
            None
        } else {
            embeddings.get(key).map(|v| embedding_to_bytes(v))
        };
        let (emb, dim, model) = match &bytes {
            Some(b) => (
                Some(b.as_slice()),
                embeddings.get(key).map(|v| v.len() as i64),
                Some(EMBEDDING_MODEL_ID),
            ),
            None => (None, None, None),
        };

        SpeakersRepository::upsert(
            pool,
            meeting_id,
            key,
            &display_name_for_key(key),
            is_local,
            emb,
            dim,
            model,
        )
        .await
        .with_context(|| format!("upsert speaker {key}"))?;
    }

    // WS3.2 (specs/0029): re-apply the snapshotted user renames onto the fresh rows.
    restore_user_identities(pool, meeting_id, renamed_snapshots, &keys, embeddings).await;

    Ok(keys.len())
}

/// Re-apply user-set speaker identities (renames / attendee assignments) after a
/// re-diarization rebuilt the `speakers` table (WS3.2, specs/0029).
///
/// Matching, in order:
/// 1. **Stable key** — sherpa's `spk_N` keys are deterministic for the same audio in
///    the common case, so a prior key that reappears in the new run carries its name
///    directly.
/// 2. **Embedding-centroid similarity** — when the key did NOT survive (different
///    clustering outcome), compare the snapshot's stored voiceprint against the new
///    run's cluster centroids and carry the name onto the best match at or above
///    [`TAU_MATCH`](crate::diarization::identity::TAU_MATCH). New keys are claimed
///    at most once so two prior names can never land on one cluster.
///
/// Limitation (documented per spec): a prior speaker with no stored voiceprint whose
/// key didn't survive is orphaned — its rename is dropped (logged). Best-effort
/// throughout: a failed row restore is logged and never fails the (already
/// persisted) diarization pass.
async fn restore_user_identities(
    pool: &sqlx::SqlitePool,
    meeting_id: &str,
    snapshots: Vec<SpeakerIdentitySnapshot>,
    new_keys: &std::collections::BTreeSet<String>,
    new_embeddings: &std::collections::HashMap<String, Vec<f32>>,
) {
    use crate::diarization::embedding::{
        cosine_similarity, embedding_from_bytes, EMBEDDING_MODEL_ID,
    };
    use crate::diarization::identity::TAU_MATCH;

    if snapshots.is_empty() {
        return;
    }

    /// One row's restore, logged either way (never propagates).
    async fn apply(
        pool: &sqlx::SqlitePool,
        meeting_id: &str,
        target_key: &str,
        snap: &SpeakerIdentitySnapshot,
        how: &str,
    ) {
        match SpeakersRepository::restore_identity(
            pool,
            meeting_id,
            target_key,
            &snap.display_name,
            snap.email.as_deref(),
            snap.person_id.as_deref(),
        )
        .await
        {
            Ok(true) => log::info!(
                "diarization: carried user speaker name '{}' onto '{target_key}' across \
                 re-run ({how}) for meeting {meeting_id} (WS3.2)",
                snap.display_name
            ),
            Ok(false) => log::warn!(
                "diarization: rename carry-over found no '{target_key}' row for meeting \
                 {meeting_id} (WS3.2)"
            ),
            Err(e) => log::warn!(
                "diarization: rename carry-over onto '{target_key}' failed for meeting \
                 {meeting_id} (continuing): {e}"
            ),
        }
    }

    // Pass 1: stable-key carry-over.
    let mut claimed: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut orphans: Vec<SpeakerIdentitySnapshot> = Vec::new();
    for snap in snapshots {
        if new_keys.contains(&snap.speaker_key) && !claimed.contains(&snap.speaker_key) {
            apply(pool, meeting_id, &snap.speaker_key, &snap, "stable key").await;
            claimed.insert(snap.speaker_key.clone());
        } else {
            orphans.push(snap);
        }
    }

    // Pass 2: centroid-similarity fallback for prior keys the new run didn't produce.
    for snap in orphans {
        let old_vec = match (snap.embedding.as_deref(), snap.embedding_model.as_deref()) {
            (Some(bytes), Some(EMBEDDING_MODEL_ID)) => match embedding_from_bytes(bytes) {
                Ok(v) => v,
                Err(e) => {
                    log::warn!(
                        "diarization: rename '{}' orphaned — stored voiceprint for prior key \
                         '{}' is unreadable ({e})",
                        snap.display_name,
                        snap.speaker_key
                    );
                    continue;
                }
            },
            _ => {
                log::info!(
                    "diarization: rename '{}' orphaned — prior key '{}' did not survive the \
                     re-run and has no comparable voiceprint (WS3.2 documented limitation)",
                    snap.display_name,
                    snap.speaker_key
                );
                continue;
            }
        };

        // Deterministic scan (sorted keys) over unclaimed new clusters.
        let mut best: Option<(&str, f32)> = None;
        for key in new_keys {
            if claimed.contains(key) {
                continue;
            }
            let Some(centroid) = new_embeddings.get(key) else {
                continue;
            };
            let sim = cosine_similarity(&old_vec, centroid);
            if best.is_none_or(|(_, b)| sim > b) {
                best = Some((key.as_str(), sim));
            }
        }

        match best {
            Some((key, sim)) if sim >= TAU_MATCH => {
                let key = key.to_string();
                apply(
                    pool,
                    meeting_id,
                    &key,
                    &snap,
                    &format!("centroid similarity {sim:.3}"),
                )
                .await;
                claimed.insert(key);
            }
            _ => log::info!(
                "diarization: rename '{}' orphaned — prior key '{}' did not survive the \
                 re-run and no new cluster matches its voiceprint (best similarity {})",
                snap.display_name,
                snap.speaker_key,
                best.map_or("n/a".to_string(), |(_, s)| format!("{s:.3}"))
            ),
        }
    }
}

/// The blocking core: ensure models, decode `system.wav`, run the diarizer.
/// Runs on a blocking thread (model download + ONNX inference + decode are sync).
///
/// Returns the speaker turns plus the per-remote-cluster L2-normalized embeddings
/// (`spk_N → vector`) for cross-meeting identity (specs/0016 1a). The embedding map
/// can be empty (extractor unavailable, all clusters too short) without failing.
/// Tell the frontend this meeting's speaker rows changed, so it refetches the transcript.
///
/// Best-effort: an emit failure means the window went away, and `EVENT_COMPLETE` is still
/// coming, so there is nothing to recover. `stage` is carried for logging/diagnostics — the
/// frontend's reaction is the same either way.
fn emit_speakers_updated<R: Runtime>(app: &AppHandle<R>, meeting_id: &str, stage: &str) {
    let _ = app.emit(
        EVENT_SPEAKERS_UPDATED,
        serde_json::json!({ "meeting_id": meeting_id, "stage": stage }),
    );
    log::info!("diarization: speakers updated for {meeting_id} at stage {stage}");
}

/// Map a diarization progress fraction (`0.0..=1.0`) to a whole-percent value for the
/// `diarization-progress` event, clamped to `0..=100` (specs/0019 WS2.5).
fn pct_from_fraction(frac: f32) -> i64 {
    (frac.clamp(0.0, 1.0) * 100.0).round() as i64
}

fn diarize_audio_blocking<R: Runtime>(
    app: AppHandle<R>,
    meeting_id: String,
    wav: PathBuf,
    speaker_count: crate::diarization::SpeakerCount,
    consolidation_floor: Option<f32>,
) -> Result<crate::diarization::TurnsWithEmbeddings> {
    // 1. Models (download on demand). Emit a coarse progress event per stage.
    let app_for_progress = app.clone();
    let mid = meeting_id.clone();
    let progress = move |stage: models::DownloadStage, downloaded: u64, total: u64| {
        let label = models::progress_label(stage, downloaded, total);
        registry_update(&mid, &label, None);
        let _ = app_for_progress.emit(
            EVENT_PROGRESS,
            serde_json::json!({ "meeting_id": mid, "stage": label }),
        );
    };
    let paths = models::ensure_models(Some(&progress)).context("ensure diarization models")?;

    registry_update(&meeting_id, "loading audio", None);
    let _ = app.emit(
        EVENT_PROGRESS,
        serde_json::json!({ "meeting_id": meeting_id, "stage": "loading audio" }),
    );

    // 2. Decode system.wav -> 16 kHz mono f32 (already 16k mono; this is a no-op
    // resample but handles any format defensively).
    //
    // First, salvage an unfinalized header: long/aborted recordings could drop the
    // per-channel writer before its WAV header was patched, leaving data-size 0 /
    // RIFF 36 even though the file holds hundreds of MB of valid PCM. symphonia trusts
    // the header → "No audio samples decoded from file". Repair is idempotent (a no-op
    // on a valid header) and best-effort — a failure here must not block decode (the
    // PCM may still be readable, and we want the clearer decode error otherwise).
    match crate::audio::channel_writer::repair_wav_header_if_needed(&wav) {
        Ok(true) => log::info!(
            "Repaired unfinalized WAV header before decode: {}",
            wav.display()
        ),
        Ok(false) => {}
        Err(e) => log::warn!(
            "WAV header repair check failed for {} (continuing): {e:#}",
            wav.display()
        ),
    }

    let decoded = crate::audio::decoder::decode_audio_file(&wav)
        .with_context(|| format!("decode {}", wav.display()))?;
    let samples = decoded.to_whisper_format();

    registry_update(&meeting_id, "diarizing", Some(0));
    let _ = app.emit(
        EVENT_PROGRESS,
        serde_json::json!({ "meeting_id": meeting_id, "stage": "diarizing", "pct": 0 }),
    );

    // 3. Diarize (auto speaker count). Provider is CoreML-with-CPU-fallback (see
    // `diarization::accel`); offline so it can use the ANE without competing with
    // live STT. We thread sherpa's chunk-progress callback up to emit a real
    // percentage, throttled to whole-percent changes so we don't spam the bridge
    // (the callback fires per audio chunk — many times a second on a long file).
    // Honor the user's "expected speakers" override (Fixed mode) when set; else
    // Auto with the tuned threshold (specs/0011 accuracy gate).
    match speaker_count {
        crate::diarization::SpeakerCount::Fixed(n) => {
            log::info!("diarization: using fixed speaker count = {n} (user override)")
        }
        crate::diarization::SpeakerCount::AtMost(n) => {
            log::info!(
                "diarization: using at-most speaker count = {n} (upper bound — auto cluster, \
                 then merge closest centroids down to <= {n})"
            )
        }
        crate::diarization::SpeakerCount::Auto => {
            log::info!("diarization: using auto speaker count (tuned threshold)")
        }
    }
    let diarizer = {
        let d = SherpaDiarizer::with_speaker_count(
            &paths.segmentation,
            &paths.embedding,
            speaker_count,
        )
        .context("init sherpa diarizer")?;
        // specs/0039 WS1: apply the optional consolidation-floor tuning override when set;
        // otherwise the diarizer uses the built-in CONSOLIDATE_FLOOR. The setter clamps
        // any override to stay above MERGE_FLOOR.
        match consolidation_floor {
            Some(f) => d.with_consolidation_floor(f),
            None => d,
        }
    };

    // specs/0016 1a: the offline pass uses `diarize_with_embeddings` (live already does)
    // so we can persist a per-remote-speaker voiceprint and run the cross-meeting matcher.
    // specs/0019 WS2.5: thread sherpa's per-chunk progress up to the UI so the
    // "Identifying speakers" indicator advances from 0% to 100% instead of sitting stuck.
    // The callback fires many times a second on a long file, so throttle emits to
    // whole-percent changes to avoid spamming the IPC bridge.
    let app_for_pct = app.clone();
    let mid_for_pct = meeting_id.clone();
    let mut last_pct: i64 = -1;
    let mut on_progress = move |frac: f32| {
        let pct = pct_from_fraction(frac);
        if pct != last_pct {
            last_pct = pct;
            registry_update(&mid_for_pct, "diarizing", Some(pct.clamp(0, 100) as u8));
            let _ = app_for_pct.emit(
                EVENT_PROGRESS,
                serde_json::json!({ "meeting_id": mid_for_pct, "stage": "diarizing", "pct": pct }),
            );
        }
    };
    let (turns, embeddings) = diarizer
        .diarize_with_embeddings_with_progress(&samples, DIARIZATION_SAMPLE_RATE, &mut on_progress)
        .context("run diarization")?;

    Ok((turns, embeddings))
}

/// The async body of `diarize_meeting` ([`crate::diarization::launch::diarize_meeting`]);
/// factored out so errors funnel to one `diarization-error` emit.
pub(super) async fn run<R: Runtime>(app: AppHandle<R>, meeting_id: String) -> Result<()> {
    log::info!("Starting diarization for meeting {meeting_id}");

    // specs/0073: hold the meeting's folder lease across every audio read (system.wav here,
    // mic.wav in `inject_owner_turns`); resolving the WAV re-reads `folder_path` under it.
    let folder_lease = crate::audio::folder_lease::acquire(
        &meeting_id,
        crate::audio::folder_lease::LeaseHolder::Diarization,
    )
    .await;
    // Resolve audio (async DB + fs check) before the blocking work.
    let wav = resolve_system_wav(&app, &meeting_id).await?;

    // Resolve the speaker count by precedence: manual override > calendar-seed >
    // Auto (specs/0011 calendar-seed). Loaded here (async) so the blocking pass
    // gets a plain value; the calendar lookup is best-effort and never fatal.
    let (speaker_count, count_source) = resolve_meeting_speaker_count(&app, &meeting_id).await;

    // specs/0039 WS1: the optional consolidation-floor tuning override (None → the
    // built-in CONSOLIDATE_FLOOR). Loaded here (async) so the blocking pass gets a
    // plain value; the diarizer clamps it above MERGE_FLOOR.
    let consolidation_floor = crate::diarization::settings::load_settings()
        .await
        .consolidation_floor;

    // Blocking: models + decode + compute (+ per-cluster re-embed for the matcher).
    let app_for_blocking = app.clone();
    let mid = meeting_id.clone();
    let (mut turns, embeddings) = tauri::async_runtime::spawn_blocking(move || {
        diarize_audio_blocking(
            app_for_blocking,
            mid,
            wav,
            speaker_count,
            consolidation_floor,
        )
    })
    .await
    .context("diarization task panicked")??;

    log::info!(
        "Diarization produced {} turns for meeting {meeting_id}",
        turns.len()
    );

    // Out of sherpa, into the DB work. Reported with no percentage (`registry_update` treats
    // `None` as indeterminate) so the button shows a named stage instead of a stuck 100%.
    registry_update(&meeting_id, STAGE_ATTRIBUTING, None);
    let _ = app.emit(
        EVENT_PROGRESS,
        serde_json::json!({ "meeting_id": meeting_id, "stage": STAGE_ATTRIBUTING }),
    );

    // Align + persist (async DB).
    let state = app.state::<AppState>();
    let pool = state.db_manager.pool();
    crate::diarization::owner_turns::inject_owner_turns(&app, pool, &meeting_id, &mut turns).await;
    drop(folder_lease); // the last audio read is done

    // specs/0044 W1.2: a fast speaker handoff (gap shorter than the VAD
    // redemption window) merges both speakers into ONE transcript row, which
    // would then take a single max-overlap label — the "first sentence on the
    // wrong speaker" bug. Split such rows at the turn boundaries BEFORE loading
    // segments for alignment. Best-effort: the split runs in its own
    // transaction, so a failure leaves rows whole and alignment still works.
    if let Err(e) = split_straddling_rows(pool, &meeting_id, &turns, SplitMode::Call).await {
        log::warn!("diarization split failed for {meeting_id} (continuing unsplit): {e:#}");
    }

    let segments = load_segments(pool, &meeting_id).await?;
    if segments.is_empty() {
        return Err(anyhow!(
            "meeting {meeting_id} has no timed transcript segments to attribute"
        ));
    }

    // specs/0029 WS3.4: channel-aware alignment. Mic-tagged segments stay "You"
    // unconditionally; system-tagged segments take diarized turns (or the unknown
    // bucket); mixed/legacy-untagged rows take the max-overlap turn, else the
    // nearest turn within 1.0s, else the unknown bucket — never "You"
    // (specs/0043 W1.3).
    // specs/0044 W1.1: overlaps are scored on the pad-trimmed speech core (the
    // stored row keeps its STT-context pads; only alignment ignores them).
    let alignable: Vec<AlignableSegment> = segments
        .iter()
        .map(|s| {
            let (ts, te) = crate::diarization::align::pad_trimmed(s.start, s.end);
            AlignableSegment::new(ts, te, s.channel)
        })
        .collect();
    let keys = align_turns_to_segments(&turns, &alignable, AlignMode::Call);
    let mic_tagged = alignable
        .iter()
        .filter(|s| s.channel == Channel::Microphone)
        .count();
    log::info!(
        "Diarization alignment for meeting {meeting_id}: {} segments ({mic_tagged} mic-tagged → \"You\" by channel)",
        alignable.len()
    );

    let assignments: Vec<(String, String)> = segments
        .into_iter()
        .zip(keys)
        .map(|(seg, key)| (seg.id, key))
        .collect();

    let persisted_count = persist(pool, &meeting_id, &assignments, &embeddings).await?;

    log::info!(
        "Diarization complete for meeting {meeting_id}: {persisted_count} speakers across {} segments",
        assignments.len()
    );

    // The clusters are committed, so the transcript can show them NOW rather than after the
    // matcher and the auto-label pass (owner feedback 2026-09-21). Emitted after the write,
    // never inside it: an event that advertises rows a rollback could still take back would
    // be worse than the wait it replaces.
    emit_speakers_updated(&app, &meeting_id, STAGE_ATTRIBUTING);

    // specs/0016 1a/1c: run the cross-meeting matcher over this meeting's freshly-
    // persisted remote voiceprints vs. previously-identified speakers AND the voiceprint
    // gallery. The meeting's calendar attendee emails corroborate gallery matches for the
    // auto-label gate. Best-effort: a matcher/DB hiccup must not fail the (already-
    // persisted) diarization pass.
    registry_update(&meeting_id, STAGE_MATCHING, None);
    let _ = app.emit(
        EVENT_PROGRESS,
        serde_json::json!({ "meeting_id": meeting_id, "stage": STAGE_MATCHING }),
    );
    let corroborating_emails: Vec<String> = lookup_calendar_attendees(&app, &meeting_id)
        .await
        .into_iter()
        .filter_map(|a| a.email)
        .map(|e| e.trim().to_lowercase())
        .filter(|e| !e.is_empty())
        .collect();
    let suggestions = compute_suggestions_with_emails(pool, &meeting_id, &corroborating_emails)
        .await
        .unwrap_or_else(|e| {
            log::warn!("diarization: cross-meeting matcher failed for {meeting_id} ({e:#}); no suggestions");
            Vec::new()
        });

    // Apply the matches confident enough to need no confirmation (specs/0016 1c,
    // specs/0044 WS4, specs/0064 W2) — persisted like a normal assignment so the user sees
    // the name immediately and the speaker is linked to the durable person. Everything else
    // stays a confirm-first chip. Shared with the on-demand refetch path, which must apply
    // them too: the gallery grows after the meetings that taught it, so a pass run before a
    // person was well-trained can only be corrected later (specs/0064 W2).
    registry_update(&meeting_id, STAGE_LABELLING, None);
    let _ = app.emit(
        EVENT_PROGRESS,
        serde_json::json!({ "meeting_id": meeting_id, "stage": STAGE_LABELLING }),
    );
    let auto_labeled = crate::diarization::auto_label::apply(pool, &meeting_id, &suggestions).await;
    // Only when something actually changed. `apply` re-derives the same labels on every
    // refetch and skips rows already settled, so a no-op pass is the common case — and it
    // must not cost the frontend a transcript refetch.
    if auto_labeled > 0 {
        emit_speakers_updated(&app, &meeting_id, STAGE_LABELLING);
    }

    // specs/0041 WS2: a summary generated before this pass finished carries no speaker
    // names (and its action-item owners come back unresolved). If this pass assigned
    // speakers AND the meeting's completed summary is speakerless and pristine
    // (not user-edited), re-run it through the same path auto-summary uses. Done BEFORE
    // the `diarization-complete` emit so that when the event (carrying
    // `summaryRefreshing: true`) reaches the frontend, the summary process row is
    // already reset to PENDING — polling can't race a stale 'completed'. Best-effort:
    // a refresh failure must never fail the (already-persisted) diarization pass.
    let summary_refreshing = match crate::summary::refresh::refresh_summary_after_diarization(
        &app,
        pool,
        &meeting_id,
        persisted_count,
    )
    .await
    {
        Ok(outcome) => outcome == crate::summary::refresh::SpeakerRefreshOutcome::Started,
        Err(e) => {
            log::warn!(
                "post-diarization summary refresh failed for meeting {meeting_id} \
                 (diarization unaffected): {e:#}"
            );
            false
        }
    };

    // `seededSpeakerCount` is the count we *forced* into the clusterer (the
    // calendar/manual estimate), distinct from `speaker_count` which is the number
    // of distinct speakers actually persisted. `speakerCountSource` tells the
    // frontend the basis ("manual"|"calendar"|"auto") so it can show e.g.
    // "Estimated N speakers from calendar".
    let seeded = match speaker_count {
        crate::diarization::SpeakerCount::Fixed(n) => Some(n),
        // specs/0017: for an upper bound the "seeded" count is the cap we asked the
        // post-cluster merge to enforce (Some(n)); 0 means "no cap" → None.
        crate::diarization::SpeakerCount::AtMost(n) => (n > 0).then_some(n),
        crate::diarization::SpeakerCount::Auto => None,
    };
    // Release the per-meeting run slot BEFORE the emit (WS3.1): a status query
    // racing the `diarization-complete` event must never read `running: true`.
    registry_finish(&meeting_id, "complete", 100);
    let _ = app.emit(
        EVENT_COMPLETE,
        serde_json::json!({
            "meeting_id": meeting_id,
            "speaker_count": persisted_count,
            "speakerCountSource": count_source.as_str(),
            // specs/0017: how the clusterer treated the count — "fixed" (exact),
            // "at_most" (cap), or "auto". Sibling to speakerCountSource (who chose it)
            // so the UI can say "Estimated up to N" for a cap vs "Forced N".
            "speakerCountMode": speaker_count.mode_str(),
            "seededSpeakerCount": seeded,
            // specs/0016 1a: name suggestions from the cross-meeting matcher (no
            // embedding bytes — display-safe only). Empty when nothing matched.
            "suggestions": suggestions,
            // specs/0041 WS2: true when this pass kicked off an automatic summary
            // regeneration to fold the new speaker names in. Informational — the summary
            // panel's "Updating with speaker names…" state is driven by the dedicated
            // `summary-refresh-started` event (emitted in summary::refresh::start_refresh),
            // which also covers the deferred wait-behind-in-flight-run path this flag
            // misses (it is false for SpeakerRefreshOutcome::DeferredBehindInFlightRun).
            "summaryRefreshing": summary_refreshing,
        }),
    );

    Ok(())
}

/// Run the cross-meeting matcher for a saved meeting: load this meeting's remote
/// voiceprints + the prior-identified candidate set (prior per-meeting speakers AND the
/// durable voiceprint-gallery centroids, specs/0016 1c), then rank by cosine via the pure
/// [`identity`](crate::diarization::identity) module. Shared by the offline pass and the
/// `api_get_speaker_suggestions` command so both produce identical results.
///
/// `corroborating_emails` (the meeting's calendar-attendee emails) is passed to the
/// matcher's auto-label gate — only a high-confidence GALLERY match whose person's email
/// is among them is marked `auto_label`. The DB-only inputs are gathered here; the ranking
/// itself stays pure.
/// With NO corroborating emails, so the email-corroborated auto-label route cannot fire —
/// the voice-only routes still can. Both real callers (the offline pass and
/// `api_get_speaker_suggestions`) supply the meeting's attendee emails via
/// [`compute_suggestions_with_emails`]; this wrapper is the email-free case used by tests.
pub async fn compute_suggestions(
    pool: &sqlx::SqlitePool,
    meeting_id: &str,
) -> Result<Vec<crate::diarization::identity::SpeakerSuggestion>> {
    compute_suggestions_with_emails(pool, meeting_id, &[]).await
}

/// As [`compute_suggestions`], but with an explicit corroborating-email set for the
/// auto-label gate. Both the offline pass and the on-demand command pass the meeting's
/// calendar attendee emails (specs/0064 W2 — the refetch APPLIES auto-labels too, so it
/// needs the same inputs the pass had; before that it deliberately passed none).
pub async fn compute_suggestions_with_emails(
    pool: &sqlx::SqlitePool,
    meeting_id: &str,
    corroborating_emails: &[String],
) -> Result<Vec<crate::diarization::identity::SpeakerSuggestion>> {
    use crate::database::repositories::people::PeopleRepository;
    use crate::database::repositories::voiceprints::VoiceprintsRepository;
    use crate::diarization::embedding::{embedding_to_bytes, EMBEDDING_MODEL_ID};
    use crate::diarization::identity::{self, CandidateSample};
    use crate::people::enroll::OWNER_PERSON_ID;

    let current = SpeakersRepository::get_meeting_embeddings(pool, meeting_id)
        .await
        .with_context(|| format!("load embeddings for meeting {meeting_id}"))?;
    if current.is_empty() {
        return Ok(Vec::new());
    }

    let identified = SpeakersRepository::get_identified_with_embeddings(pool)
        .await
        .context("load identified candidate speakers")?;

    // 1a candidates: prior per-meeting speaker embeddings (exclude THIS meeting's own
    // rows so a speaker is never suggested against itself, and any row linked to the
    // owner — see the W1.4 note below).
    let mut candidates: Vec<CandidateSample> = identified
        .into_iter()
        .filter(|c| c.meeting_id != meeting_id && c.person_id.as_deref() != Some(OWNER_PERSON_ID))
        .map(|c| CandidateSample {
            display_name: c.display_name,
            email: c.email,
            person_id: c.person_id,
            embedding: c.embedding,
            embedding_model: c.embedding_model,
            from_gallery: false,
            gallery_sample_count: 0,
            // The meeting this prior speaker row belongs to — the matcher counts DISTINCT
            // meetings for the repetition tier (specs/0064 W2 review).
            meeting_id: Some(c.meeting_id),
        })
        .collect();

    // 1c candidates: durable voiceprint-gallery centroids, enriched with each person's
    // name/email so the matcher can corroborate + label. Best-effort — a gallery read
    // failure must not sink the (1a) suggestions.
    match VoiceprintsRepository::all_centroids(pool, EMBEDDING_MODEL_ID).await {
        Ok(centroids) => {
            for (person_id, centroid, sample_count) in centroids {
                // W1.4 (specs/0043): the owner never competes in the gallery. "You" is
                // assigned via the mic channel (align.rs) — a similarity hit here could
                // only mislabel a remote cluster (e.g. echo bleed) as the owner.
                if person_id == OWNER_PERSON_ID {
                    continue;
                }
                let (name, email) = match PeopleRepository::get(pool, &person_id).await {
                    Ok(Some(p)) => (p.display_name, p.email),
                    // Centroid with no person row (shouldn't happen — explicit cascades);
                    // skip rather than label with a placeholder.
                    _ => continue,
                };
                candidates.push(CandidateSample {
                    display_name: name,
                    email,
                    person_id: Some(person_id),
                    embedding: embedding_to_bytes(&centroid),
                    embedding_model: Some(EMBEDDING_MODEL_ID.to_string()),
                    from_gallery: true,
                    // specs/0044 WS4: enrollment count gates the trusted (voice-only)
                    // auto-label tier in the matcher.
                    gallery_sample_count: sample_count,
                    // A centroid belongs to no single meeting.
                    meeting_id: None,
                });
            }
        }
        Err(e) => log::warn!(
            "diarization: gallery centroid load failed for meeting {meeting_id} ({e}); \
             matching prior speakers only"
        ),
    }

    Ok(identity::match_speakers_with_gallery(
        &current,
        &candidates,
        corroborating_emails,
    ))
}

/// Resolve the [`SpeakerCount`](crate::diarization::SpeakerCount) for a meeting by
/// precedence (manual > calendar-remote > Auto), returning the source for the UI.
///
/// Thin, best-effort wrapper around the pure
/// [`resolve_speaker_count`](crate::diarization::settings::resolve_speaker_count):
/// loads settings, and — only when there's no manual override — sizes the cap.
///
/// Sizing is **roster-first** (specs/0017): the persistent participant roster
/// ([`MeetingParticipantsRepository::remote_person_ids`]) is consulted first, so a
/// user's manual add/remove of participants moves the cap. Each remote participant
/// becomes one unit of the `AtMost(n)` upper bound (source [`Calendar`]). Only when
/// the roster is empty do we fall back to the live EventKit attendee lookup (via the
/// SAME reader the attendee-mapping command uses), which also now yields `AtMost`.
///
/// Both the roster query and the calendar lookup are **non-fatal**: a DB error,
/// denied access, no matching event, a missing meeting row, or any error just yields
/// no count, falling through to Auto. Diarization must never fail because of the
/// roster or the calendar.
///
/// [`Calendar`]: crate::diarization::settings::SpeakerCountSource::Calendar
async fn resolve_meeting_speaker_count<R: Runtime>(
    app: &AppHandle<R>,
    meeting_id: &str,
) -> (
    crate::diarization::SpeakerCount,
    crate::diarization::settings::SpeakerCountSource,
) {
    use crate::diarization::settings;

    // specs/0017 roster-first sizing: the persistent participant roster is the
    // user-curatable source of truth for the cap — manual add/remove moves it.
    // WS3.3 (specs/0029): the bound uses the remote-linked person count when
    // available, but falls back to the FULL roster row count so hand-added
    // participants still cap the run (previously an unlinked roster fell all the
    // way through to uncapped Auto → 25 "speakers" on a 5-person meeting).
    // Best-effort: any DB error → fall through to the live calendar lookup → Auto.
    let state = app.state::<AppState>();
    let pool = state.db_manager.pool();
    let remote_count = crate::database::repositories::meeting_participant::MeetingParticipantsRepository::remote_person_ids(
        pool, meeting_id,
    )
    .await
    .map(|ids| ids.len())
    .unwrap_or_else(|e| {
        log::warn!(
            "diarization: could not load remote roster participants for meeting \
             {meeting_id} ({e}); trying the full roster count"
        );
        0
    });
    let total_count = crate::database::repositories::meeting_participant::MeetingParticipantsRepository::count_for_meeting(
        pool, meeting_id,
    )
    .await
    .unwrap_or_else(|e| {
        log::warn!(
            "diarization: could not count roster participants for meeting {meeting_id} \
             ({e}); falling back to calendar lookup"
        );
        0
    });
    if let Some(n) = roster_speaker_cap(remote_count, total_count) {
        log::info!(
            "diarization: capping speakers at AtMost({n}) from the persistent roster \
             ({remote_count} remote-linked, {total_count} total participants) for \
             meeting {meeting_id}"
        );
        return (
            crate::diarization::SpeakerCount::AtMost(n),
            settings::SpeakerCountSource::Calendar,
        );
    }

    // Best-effort calendar lookup (roster was empty). EventKit touches the Objective-C
    // runtime, so it runs off the async executor (mirrors `api_get_meeting_attendees`).
    // Any failure collapses to an empty attendee list → Auto.
    let attendees = lookup_calendar_attendees(app, meeting_id).await;

    let total = attendees.len();
    let (count, source) = settings::resolve_speaker_count(&attendees);
    match source {
        settings::SpeakerCountSource::Calendar => {
            if let crate::diarization::SpeakerCount::AtMost(n) = count {
                log::info!(
                    "diarization: capping speakers at AtMost({n}) from {total} calendar \
                     attendees (excluding self) for meeting {meeting_id}"
                );
            }
        }
        settings::SpeakerCountSource::Auto => {
            log::info!(
                "diarization: no reliable seed for meeting {meeting_id} ({total} attendees) — \
                 Auto; the audio seed caps at AtMost(n_audio) in the pass (specs/0050)"
            );
        }
    }
    (count, source)
}

/// The roster-derived `AtMost` bound (WS3.3, specs/0029). Pure so the precedence is
/// unit-testable without an `AppHandle`:
/// - Prefer the **remote-linked** participant count (owner excluded — diarization
///   clusters the system channel only, so "You" never inflates the cap).
/// - Fall back to the **full roster row count** when no remote linkage exists (the
///   common case for hand-added participants), so a curated roster still bounds
///   the run instead of falling through to uncapped Auto.
/// - `None` when the roster is empty → caller falls through to calendar → Auto.
fn roster_speaker_cap(remote_count: usize, total_count: i64) -> Option<u32> {
    let n = if remote_count > 0 {
        remote_count
    } else {
        usize::try_from(total_count).unwrap_or(0)
    };
    (n > 0).then(|| n.min(u32::MAX as usize) as u32)
}

/// Look up the linked calendar event's attendees for a saved meeting. Best-effort:
/// returns an empty `Vec` on any failure (no meeting row, EventKit denied/error, no
/// matching event) and logs at info/warn — never propagates an error, so the caller
/// can always fall through to Auto.
pub(crate) async fn lookup_calendar_attendees<R: Runtime>(
    app: &AppHandle<R>,
    meeting_id: &str,
) -> Vec<crate::calendar::eventkit::Attendee> {
    let state = app.state::<AppState>();
    let pool = state.db_manager.pool();

    let meta =
        match crate::database::repositories::meeting::MeetingsRepository::get_meeting_metadata(
            pool, meeting_id,
        )
        .await
        {
            Ok(Some(m)) => m,
            Ok(None) => {
                log::info!(
                    "diarization: meeting {meeting_id} not found for calendar seed; using Auto"
                );
                return Vec::new();
            }
            Err(e) => {
                log::warn!(
                    "diarization: could not load meeting {meeting_id} for calendar seed ({e:#}); \
                 using Auto"
                );
                return Vec::new();
            }
        };

    // Google-sourced meetings (specs/0032): a `gcal:` calendar_event_id routes
    // straight to the local cache row's attendees — no EventKit, no network.
    // After a disconnect the purged cache degrades this to empty (Auto).
    if let Some(event_id) = meta.calendar_event_id.as_deref() {
        if event_id.starts_with("gcal:") {
            return crate::calendar::google::sync::cached_attendees_for_event_id(pool, event_id)
                .await;
        }
    }

    let title = meta.title.clone();
    let started_at = meta.created_at.0.to_rfc3339();

    // Title+time fallback, routed by the single active source (specs/0032):
    // Google connected → local-cache lookup only; otherwise EventKit only.
    if crate::calendar::google_is_active_source(app).await {
        return crate::calendar::google::sync::cached_attendees_by_title_time(
            pool,
            &title,
            &started_at,
        )
        .await;
    }
    match tokio::task::spawn_blocking(move || {
        crate::calendar::eventkit::event_attendees(&title, &started_at)
    })
    .await
    {
        Ok(attendees) => attendees,
        Err(e) => {
            log::warn!(
                "diarization: calendar attendee lookup task failed for meeting \
                 {meeting_id} ({e}); using Auto"
            );
            Vec::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_names_map_local_and_clusters() {
        assert_eq!(display_name_for_key("local"), "You");
        assert_eq!(display_name_for_key("spk_0"), "Speaker 1");
        assert_eq!(display_name_for_key("spk_1"), "Speaker 2");
        assert_eq!(display_name_for_key("spk_9"), "Speaker 10");
        // WS3.3 overflow bucket (specs/0029).
        assert_eq!(display_name_for_key("unknown"), "Unknown speaker");
        // Unrecognized/fallback keys pass through.
        assert_eq!(display_name_for_key("spk_unknown"), "spk_unknown");
    }

    // -----------------------------------------------------------------------
    // WS3.1 (specs/0029) — per-meeting in-flight run registry.
    // -----------------------------------------------------------------------

    #[test]
    fn run_registry_guards_updates_and_releases() {
        // Unique id: the registry is a process-global, tests run in parallel.
        let id = "ws31-registry-test-meeting";
        assert!(run_status(id).is_none(), "no status before any run");

        assert!(registry_try_begin(id), "first begin claims the slot");
        assert!(
            !registry_try_begin(id),
            "second begin is refused while running"
        );

        let s = run_status(id).expect("live status");
        assert!(s.running);
        assert_eq!(s.stage, "starting");
        assert_eq!(s.progress_pct, 0);

        registry_update(id, "diarizing", Some(42));
        let s = run_status(id).unwrap();
        assert_eq!((s.stage.as_str(), s.progress_pct), ("diarizing", 42));

        // A stage change resets the percentage (each stage owns its own ramp).
        registry_update(id, "loading audio", None);
        let s = run_status(id).unwrap();
        assert_eq!((s.stage.as_str(), s.progress_pct), ("loading audio", 0));

        // Terminal status stays queryable and the slot is released.
        registry_finish(id, "complete", 100);
        let s = run_status(id).unwrap();
        assert!(!s.running);
        assert_eq!((s.stage.as_str(), s.progress_pct), ("complete", 100));

        // Updates after the run ended are ignored (a straggling callback).
        registry_update(id, "diarizing", Some(7));
        assert_eq!(run_status(id).unwrap().stage, "complete");

        assert!(registry_try_begin(id), "slot reusable after terminal state");
        registry_finish(id, "error", 0);
        assert!(!run_status(id).unwrap().running);
    }

    // -----------------------------------------------------------------------
    // Owner feedback 2026-09-21 — the run reports what it is doing after sherpa returns.
    //
    // Before this, the only stages were models / loading audio / diarizing, so from the
    // moment sherpa hit 100% the button read "Identifying… 100%" through alignment,
    // persistence, the cross-meeting matcher and auto-labelling. A static 100% reads as
    // hung, which is the same reasoning specs/0024 WS4.1 applied to a static 0%.
    // -----------------------------------------------------------------------

    #[test]
    fn post_sherpa_stages_clear_the_stuck_percentage() {
        // Unique id: the registry is a process-global and tests run in parallel.
        let id = "post-sherpa-stage-test-meeting";
        assert!(registry_try_begin(id));

        // Sherpa ramps to 100 …
        registry_update(id, "diarizing", Some(100));
        assert_eq!(run_status(id).unwrap().progress_pct, 100);

        // … and then each post-pass stage reports itself with NO percentage, which
        // `registry_update` stores as 0 and the frontend renders as a bare stage label.
        for stage in [STAGE_ATTRIBUTING, STAGE_MATCHING, STAGE_LABELLING] {
            registry_update(id, stage, None);
            let s = run_status(id).unwrap();
            assert_eq!(s.stage, stage, "stage is reported");
            assert_eq!(s.progress_pct, 0, "{stage} must not inherit sherpa's 100%");
            assert!(s.running, "the run is still going");
        }

        registry_finish(id, "complete", 100);
    }

    /// The stage strings are the frontend's only handle on what is happening, and they are
    /// rendered verbatim into the button ("Matching known voices…"). Pin them so a rename
    /// here has to be a deliberate, visible change.
    #[test]
    fn post_sherpa_stage_names_are_human_readable() {
        assert_eq!(STAGE_ATTRIBUTING, "attributing");
        assert_eq!(STAGE_MATCHING, "matching known voices");
        assert_eq!(STAGE_LABELLING, "labelling");
    }

    // -----------------------------------------------------------------------
    // WS3.3 (specs/0029) — roster-informed speaker cap.
    // -----------------------------------------------------------------------

    #[test]
    fn roster_cap_prefers_remote_then_falls_back_to_total() {
        // Remote-linked participants win when present.
        assert_eq!(roster_speaker_cap(3, 5), Some(3));
        // No remote linkage → the FULL roster count still bounds the run
        // (the 25-speakers-on-a-5-person-roster regression).
        assert_eq!(roster_speaker_cap(0, 5), Some(5));
        // Empty roster → no bound (caller falls through to calendar → Auto).
        assert_eq!(roster_speaker_cap(0, 0), None);
        // Defensive: a negative count (impossible from COUNT(*)) is no bound.
        assert_eq!(roster_speaker_cap(0, -1), None);
    }

    // -----------------------------------------------------------------------
    // WS3.2 (specs/0029) — user renames survive a re-diarization.
    // -----------------------------------------------------------------------

    use sqlx::sqlite::SqlitePoolOptions;
    use sqlx::SqlitePool;
    use std::collections::HashMap;

    /// In-memory pool with just the tables `persist` touches (mirrors the
    /// migration shape; no `PRAGMA foreign_keys`, like the app pool).
    async fn test_pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("open in-memory sqlite");

        sqlx::query(
            "CREATE TABLE transcripts (
                id TEXT PRIMARY KEY,
                meeting_id TEXT NOT NULL,
                speaker TEXT,
                audio_start_time REAL,
                audio_end_time REAL,
                timestamp TEXT
            )",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "CREATE TABLE speakers (
                id TEXT PRIMARY KEY,
                meeting_id TEXT NOT NULL,
                speaker_key TEXT NOT NULL,
                display_name TEXT NOT NULL,
                is_local INTEGER NOT NULL DEFAULT 0,
                email TEXT,
                person_id TEXT,
                embedding BLOB,
                embedding_dim INTEGER,
                embedding_model TEXT,
                created_at TEXT,
                updated_at TEXT,
                UNIQUE(meeting_id, speaker_key)
            )",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "CREATE TABLE transcript_speaker_overrides (
                transcript_id TEXT PRIMARY KEY,
                meeting_id TEXT NOT NULL,
                speaker_key TEXT NOT NULL,
                created_at TEXT
            )",
        )
        .execute(&pool)
        .await
        .unwrap();

        // Two timed transcript segments for meeting m1.
        for (id, s, e) in [("t1", 0.0, 4.0), ("t2", 4.0, 9.0)] {
            sqlx::query(
                "INSERT INTO transcripts (id, meeting_id, speaker, audio_start_time, audio_end_time)
                 VALUES (?, 'm1', NULL, ?, ?)",
            )
            .bind(id)
            .bind(s)
            .bind(e)
            .execute(&pool)
            .await
            .unwrap();
        }
        pool
    }

    fn assignments(remote_key: &str) -> Vec<(String, String)> {
        vec![
            ("t1".to_string(), "local".to_string()),
            ("t2".to_string(), remote_key.to_string()),
        ]
    }

    async fn display_name(pool: &SqlitePool, key: &str) -> Option<String> {
        sqlx::query_scalar::<_, String>(
            "SELECT display_name FROM speakers WHERE meeting_id = 'm1' AND speaker_key = ?",
        )
        .bind(key)
        .fetch_optional(pool)
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn rename_survives_rerun_when_key_is_stable() {
        let pool = test_pool().await;
        let emb: HashMap<String, Vec<f32>> =
            HashMap::from([("spk_0".to_string(), vec![1.0, 0.0, 0.0, 0.0])]);

        persist(&pool, "m1", &assignments("spk_0"), &emb)
            .await
            .unwrap();
        assert_eq!(
            display_name(&pool, "spk_0").await.as_deref(),
            Some("Speaker 1")
        );

        // User renames the diarized speaker...
        assert!(SpeakersRepository::rename(&pool, "m1", "spk_0", "Priya")
            .await
            .unwrap());

        // ...then a re-run persists the same clustering. The rename must survive.
        persist(&pool, "m1", &assignments("spk_0"), &emb)
            .await
            .unwrap();
        assert_eq!(display_name(&pool, "spk_0").await.as_deref(), Some("Priya"));
        // The untouched local row keeps its generated default.
        assert_eq!(display_name(&pool, "local").await.as_deref(), Some("You"));
    }

    #[tokio::test]
    async fn rename_survives_rerun_via_centroid_when_key_changes() {
        let pool = test_pool().await;
        let voice = vec![0.8, 0.6, 0.0, 0.0]; // L2-normalized
        let emb1: HashMap<String, Vec<f32>> = HashMap::from([("spk_0".to_string(), voice.clone())]);

        persist(&pool, "m1", &assignments("spk_0"), &emb1)
            .await
            .unwrap();
        SpeakersRepository::rename(&pool, "m1", "spk_0", "Priya")
            .await
            .unwrap();

        // Re-run clusters the same voice under a DIFFERENT key. The stored
        // voiceprint must carry the rename onto the new key.
        let emb2: HashMap<String, Vec<f32>> = HashMap::from([("spk_1".to_string(), voice)]);
        persist(&pool, "m1", &assignments("spk_1"), &emb2)
            .await
            .unwrap();

        assert_eq!(display_name(&pool, "spk_1").await.as_deref(), Some("Priya"));
        // The old key's row is gone (cleared by the re-run).
        assert_eq!(display_name(&pool, "spk_0").await, None);
    }

    #[tokio::test]
    async fn attendee_assignment_survives_rerun() {
        let pool = test_pool().await;
        let emb: HashMap<String, Vec<f32>> =
            HashMap::from([("spk_0".to_string(), vec![0.0, 1.0, 0.0, 0.0])]);

        persist(&pool, "m1", &assignments("spk_0"), &emb)
            .await
            .unwrap();
        assert!(SpeakersRepository::assign_to_attendee(
            &pool,
            "m1",
            "spk_0",
            "Priya Patel",
            "priya@acme.io"
        )
        .await
        .unwrap());

        persist(&pool, "m1", &assignments("spk_0"), &emb)
            .await
            .unwrap();

        let row = sqlx::query_as::<_, (String, Option<String>)>(
            "SELECT display_name, email FROM speakers
             WHERE meeting_id = 'm1' AND speaker_key = 'spk_0'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(row.0, "Priya Patel");
        assert_eq!(row.1.as_deref(), Some("priya@acme.io"));
    }

    #[tokio::test]
    async fn dissimilar_voice_does_not_steal_a_rename() {
        let pool = test_pool().await;
        let emb1: HashMap<String, Vec<f32>> =
            HashMap::from([("spk_0".to_string(), vec![1.0, 0.0, 0.0, 0.0])]);

        persist(&pool, "m1", &assignments("spk_0"), &emb1)
            .await
            .unwrap();
        SpeakersRepository::rename(&pool, "m1", "spk_0", "Priya")
            .await
            .unwrap();

        // Re-run produces a different key AND an orthogonal (different-voice)
        // centroid: below TAU_MATCH the rename must be dropped, not misapplied.
        let emb2: HashMap<String, Vec<f32>> =
            HashMap::from([("spk_1".to_string(), vec![0.0, 0.0, 1.0, 0.0])]);
        persist(&pool, "m1", &assignments("spk_1"), &emb2)
            .await
            .unwrap();

        assert_eq!(
            display_name(&pool, "spk_1").await.as_deref(),
            Some("Speaker 2"),
            "an orphaned rename must never land on a dissimilar voice"
        );
    }

    // -----------------------------------------------------------------------
    // W1.4 (specs/0043) — the owner never competes in the suggestion gallery.
    // -----------------------------------------------------------------------

    /// [`test_pool`] plus the `people`/`voiceprints` tables `compute_suggestions`
    /// reads (mirrors the migration shape, minimal columns).
    async fn suggestion_pool() -> SqlitePool {
        let pool = test_pool().await;
        for sql in [
            "CREATE TABLE people (
                id TEXT PRIMARY KEY, email TEXT, display_name TEXT NOT NULL,
                role TEXT, notes TEXT, voiceprint_opt_out INTEGER NOT NULL DEFAULT 0,
                starred INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL, updated_at TEXT NOT NULL)",
            "CREATE TABLE voiceprints (
                id TEXT PRIMARY KEY, person_id TEXT NOT NULL, embedding BLOB NOT NULL,
                embedding_dim INTEGER NOT NULL, embedding_model TEXT NOT NULL,
                source_meeting_id TEXT, source_speaker_key TEXT, sample_quality REAL,
                quarantined_at TEXT, created_at TEXT NOT NULL)",
            // specs/0056 W6: `PeopleRepository::get` LEFT JOINs the photo cache.
            "CREATE TABLE attendee_photos (
                email TEXT PRIMARY KEY, photo_data_uri TEXT NOT NULL, fetched_at TEXT NOT NULL)",
        ] {
            sqlx::query(sql).execute(&pool).await.unwrap();
        }
        pool
    }

    async fn insert_person(pool: &SqlitePool, id: &str, name: &str, email: Option<&str>) {
        sqlx::query(
            "INSERT INTO people (id, email, display_name, created_at, updated_at)
             VALUES (?, ?, ?, '2026-01-01', '2026-01-01')",
        )
        .bind(id)
        .bind(email)
        .bind(name)
        .execute(pool)
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn owner_is_excluded_from_speaker_suggestions() {
        use crate::database::repositories::voiceprints::VoiceprintsRepository;
        use crate::diarization::embedding::{embedding_to_bytes, EMBEDDING_MODEL_ID};
        use crate::people::enroll::OWNER_PERSON_ID;

        let pool = suggestion_pool().await;

        // Current meeting m1: one remote cluster whose voice is IDENTICAL to the
        // owner's stored voiceprint (e.g. the owner's echo bleeding onto the system
        // channel) and merely similar (cos 0.8, still >= TAU_MATCH) to Priya's.
        let voice = vec![1.0, 0.0, 0.0, 0.0];
        let emb: HashMap<String, Vec<f32>> = HashMap::from([("spk_0".to_string(), voice.clone())]);
        persist(&pool, "m1", &assignments("spk_0"), &emb)
            .await
            .unwrap();

        insert_person(&pool, OWNER_PERSON_ID, "You", None).await;
        insert_person(&pool, "p-priya", "Priya", Some("priya@acme.io")).await;

        // Gallery (1c) centroids for BOTH people. Unfiltered, the owner (cos 1.0)
        // would beat Priya (cos 0.8) and win the suggestion.
        VoiceprintsRepository::add_sample(
            &pool,
            OWNER_PERSON_ID,
            &voice,
            EMBEDDING_MODEL_ID,
            None,
            None,
            Some(1.0),
        )
        .await
        .unwrap();
        VoiceprintsRepository::add_sample(
            &pool,
            "p-priya",
            &[0.8, 0.6, 0.0, 0.0],
            EMBEDDING_MODEL_ID,
            None,
            None,
            Some(1.0),
        )
        .await
        .unwrap();

        // A prior-meeting (1a) speaker row linked to the owner must be filtered too.
        sqlx::query(
            "INSERT INTO speakers (id, meeting_id, speaker_key, display_name, is_local,
                                   person_id, embedding, embedding_dim, embedding_model)
             VALUES ('s-owner-m0', 'm0', 'spk_0', 'You', 0, ?, ?, 4, ?)",
        )
        .bind(OWNER_PERSON_ID)
        .bind(embedding_to_bytes(&voice))
        .bind(EMBEDDING_MODEL_ID)
        .execute(&pool)
        .await
        .unwrap();

        let suggestions = compute_suggestions(&pool, "m1").await.unwrap();

        assert!(
            suggestions
                .iter()
                .all(|s| s.suggested_person_id.as_deref() != Some(OWNER_PERSON_ID)),
            "a remote cluster must never be suggested as the owner"
        );
        // With the owner out of the field, the normal person still gets suggested.
        let s = suggestions
            .iter()
            .find(|s| s.speaker_key == "spk_0")
            .expect("the non-owner person is still suggested");
        assert_eq!(s.suggested_person_id.as_deref(), Some("p-priya"));
        assert_eq!(s.suggested_name, "Priya");
    }

    #[test]
    fn pct_from_fraction_maps_and_clamps() {
        // specs/0019 WS2.5 — the "Identifying speakers" progress maps 0..1 -> 0..100.
        assert_eq!(pct_from_fraction(0.0), 0);
        assert_eq!(pct_from_fraction(0.25), 25);
        assert_eq!(pct_from_fraction(0.5), 50);
        assert_eq!(pct_from_fraction(1.0), 100);
        // Clamped at both ends (a misbehaving callback can't push it out of range).
        assert_eq!(pct_from_fraction(-0.3), 0);
        assert_eq!(pct_from_fraction(1.7), 100);
        assert_eq!(pct_from_fraction(0.994), 99);
        assert_eq!(pct_from_fraction(0.996), 100);
    }
}
