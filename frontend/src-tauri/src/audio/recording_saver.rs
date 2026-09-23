use anyhow::Result;
use log::{error, info, warn};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, Runtime};
use tokio::sync::mpsc;
use tokio::sync::Mutex as AsyncMutex;

use super::audio_processing::create_meeting_folder;
use super::channel_files;
use super::channel_writer::{MIC_CHANNEL_FILENAME, SYSTEM_CHANNEL_FILENAME};
use super::ffmpeg::find_ffmpeg_path;
use super::incremental_saver::IncrementalAudioSaver;
use super::recording_state::AudioChunk;

/// Canonical merged meeting audio filename (single-segment output; also the concat
/// target when a meeting has multiple segments).
const MEETING_AUDIO_FILENAME: &str = "audio.mp4";

/// Seconds of audio per incremental checkpoint chunk — mirrors
/// `IncrementalAudioSaver`'s `sample_rate * 30` flush interval. Used only as a coarse
/// fallback when a crash-recovered segment's real duration can't be probed (specs/0037).
const CHECKPOINT_SECONDS: f64 = 30.0;

/// Structured transcript segment for JSON export
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscriptSegment {
    pub id: String,
    pub text: String,
    pub audio_start_time: f64, // Seconds from recording start
    pub audio_end_time: f64,   // Seconds from recording start
    pub duration: f64,         // Segment duration in seconds
    pub display_time: String,  // Formatted time for display like "[02:15]"
    pub confidence: f32,
    pub sequence_id: u64,
    /// specs/0029 WS3.4 follow-up: capture-channel tag ("microphone" | "system" |
    /// "mixed"), carried in the rehydration copy so a page reload mid-recording
    /// doesn't lose channel attribution (the frontend threads it back into the
    /// save payload → `transcripts.channel`). Defaulted for older transcripts.json.
    #[serde(default)]
    pub channel: Option<String>,
}

/// One recording session inside a meeting folder (specs/0037). Session #0 is the
/// initial recording; each "Continue recording" / crash-resume appends one more. The
/// per-segment audio files are concatenated into the canonical `audio.mp4` /
/// `system.wav` / `mic.wav` at final stop so the offline diarization pass (which reads a
/// single `system.wav` and full-recomputes) covers the whole meeting.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordingSegment {
    pub index: u32,
    pub started_at: String,
    pub completed_at: Option<String>,
    pub duration_seconds: Option<f64>,
    /// Segment-scoped audio filename (`audio.mp4` for a lone session #0, else
    /// `audio_seg{NN}.mp4`), relative to the meeting folder.
    pub audio_file: String,
    /// Segment-scoped system-channel WAV (`system.wav` for a lone session #0, else
    /// `system_seg{NN}.wav`), relative to the meeting folder.
    pub system_wav: String,
    /// Segment-scoped mic-channel WAV (`mic.wav` for a lone session #0, else
    /// `mic_seg{NN}.wav`), relative to the meeting folder.
    pub mic_wav: String,
}

/// Meeting metadata structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeetingMetadata {
    pub version: String,
    pub meeting_id: Option<String>,
    pub meeting_name: Option<String>,
    pub created_at: String,
    pub completed_at: Option<String>,
    pub duration_seconds: Option<f64>,
    pub devices: DeviceInfo,
    pub audio_file: String,
    pub transcript_file: String,
    pub sample_rate: u32,
    pub status: String, // "recording", "completed", "error"
    /// Recording sessions that make up this meeting (specs/0037). `#[serde(default)]`
    /// keeps pre-0037 `metadata.json` (no `segments` key) deserializable — those load as
    /// an empty vec. A first/only session records `segments[0]` at stop.
    #[serde(default)]
    pub segments: Vec<RecordingSegment>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceInfo {
    pub microphone: Option<String>,
    pub system_audio: Option<String>,
}

/// New recording saver using incremental saving strategy
pub struct RecordingSaver {
    incremental_saver: Option<Arc<AsyncMutex<IncrementalAudioSaver>>>,
    meeting_folder: Option<PathBuf>,
    meeting_name: Option<String>,
    metadata: Option<MeetingMetadata>,
    transcript_segments: Arc<Mutex<Vec<TranscriptSegment>>>,
    chunk_receiver: Option<mpsc::UnboundedReceiver<AudioChunk>>,
    is_saving: Arc<Mutex<bool>>,
    /// DB meeting id to persist into `metadata.json` at init (specs/0037). Previously
    /// hardcoded to `None`; linking the folder to its row enables resume + crash recovery.
    meeting_id: Option<String>,
    /// When `Some`, RESUME mode (specs/0037): reuse this existing meeting folder instead
    /// of creating a fresh one, and append a new segment.
    resume_folder: Option<PathBuf>,
    /// Index of the segment THIS session writes: `0` for a fresh recording, the next
    /// free index (past every existing AND crash-recovered segment) when resuming.
    segment_index: u32,
    /// RFC3339 timestamp when this session's accumulation started (segment `started_at`).
    segment_started_at: Option<String>,
    /// Audio offset (seconds) for this session = sum of prior segments' durations. `0.0`
    /// for a fresh recording; the resume-mode transcript-time offset the command layer
    /// applies. Fixed at init; unaffected by this session's own segment.
    prior_audio_duration: f64,
    /// Transcript segments loaded from the folder's existing `transcripts.json` at
    /// resume init (specs/0037). `write_transcripts_json` prepends these before the
    /// current session's segments so the resumed session's first incremental write can
    /// never destroy the crashed/prior session's only on-disk transcript copy. Kept OUT
    /// of `transcript_segments` deliberately: `get_transcript_segments()` feeds the
    /// reload-sync path and must return only the CURRENT session.
    prior_transcript_segments: Vec<TranscriptSegment>,
    /// Journal of the destructive on-disk mutations `initialize_resume_folder` performed
    /// (specs/0037). Kept until the caller confirms the start succeeded
    /// ([`commit_start`](Self::commit_start)); on a failed start
    /// [`rollback_failed_start`](Self::rollback_failed_start) replays it in reverse so a
    /// cleanly-completed folder is left exactly as it was (status `"completed"`,
    /// canonical filenames, no phantom `.checkpoints/`).
    resume_init_journal: Option<ResumeInitJournal>,
    /// specs/0073: the meeting's folder lease, held from start through finalization.
    folder_lease: Option<super::folder_lease::FolderLease>,
}

/// What `initialize_resume_folder` changed on disk, so a failed start can be rolled
/// back (specs/0037 FIX: a start that fails AFTER resume init previously left the
/// folder destructively mutated — status flipped to "recording", canonical
/// `audio.mp4`/`system.wav`/`mic.wav` renamed away, and an empty `.checkpoints/`
/// causing a phantom "Unfinished recording" offer at every launch).
///
/// NOT journaled (deliberately, and safe): crash-recovery checkpoint merges. Those
/// consume loose chunks into `audio_seg{NN}.mp4`, which cannot be un-merged — but the
/// merge is pure recovery (never session-specific), and `migrate_prior_segments` is
/// self-healing about it on the next resume: it adopts orphan `audio_seg{NN}.mp4`
/// files and re-probes missing durations, so restoring the ORIGINAL `metadata.json`
/// bytes here still yields a correct next resume.
struct ResumeInitJournal {
    folder: PathBuf,
    /// Raw original `metadata.json` contents, restored verbatim on rollback.
    original_metadata_json: String,
    /// `(original, renamed_to)` pairs, undone in reverse order on rollback.
    renames: Vec<(PathBuf, PathBuf)>,
    /// Whether `.checkpoints/` was created by this init (it did not exist before). Only
    /// then is the (empty) directory removed on rollback — a pre-existing dir may hold a
    /// crashed session's preserved chunks and also keeps the folder offerable for
    /// resume on the next launch.
    created_checkpoints_dir: bool,
}

impl ResumeInitJournal {
    /// Best-effort inverse of the journaled mutations. Never fails; logs on error.
    fn rollback(&self) {
        for (original, renamed_to) in self.renames.iter().rev() {
            if renamed_to.exists() && !original.exists() {
                if let Err(e) = std::fs::rename(renamed_to, original) {
                    warn!(
                        "Rollback: failed to restore {} → {}: {}",
                        renamed_to.display(),
                        original.display(),
                        e
                    );
                }
            }
        }

        // Restore the original metadata.json verbatim (atomic write, same as
        // write_metadata).
        let metadata_path = self.folder.join("metadata.json");
        let temp_path = self.folder.join(".metadata.json.tmp");
        let restored = std::fs::write(&temp_path, &self.original_metadata_json)
            .and_then(|_| std::fs::rename(&temp_path, &metadata_path));
        if let Err(e) = restored {
            warn!(
                "Rollback: failed to restore original metadata.json in {}: {}",
                self.folder.display(),
                e
            );
        }

        if self.created_checkpoints_dir {
            // Only removable when empty — preserved chunks (if any appeared) survive.
            let _ = std::fs::remove_dir(self.folder.join(".checkpoints"));
        }
        info!(
            "Rolled back resume init for folder {}",
            self.folder.display()
        );
    }
}

impl RecordingSaver {
    pub fn new() -> Self {
        Self {
            incremental_saver: None,
            meeting_folder: None,
            meeting_name: None,
            metadata: None,
            transcript_segments: Arc::new(Mutex::new(Vec::new())),
            chunk_receiver: None,
            is_saving: Arc::new(Mutex::new(false)),
            meeting_id: None,
            resume_folder: None,
            segment_index: 0,
            segment_started_at: None,
            prior_audio_duration: 0.0,
            prior_transcript_segments: Vec::new(),
            resume_init_journal: None,
            folder_lease: None,
        }
    }

    /// Hand the saver its meeting's folder lease (taken by the start command); released when
    /// finalization ends (`stop_and_save`), a start is rolled back, or the saver drops.
    pub fn set_folder_lease(&mut self, lease: Option<super::folder_lease::FolderLease>) {
        self.folder_lease = lease;
    }

    /// Set the meeting name for this recording session
    pub fn set_meeting_name(&mut self, name: Option<String>) {
        self.meeting_name = name;
    }

    /// Set the DB meeting id (specs/0037). Stored and written into `metadata.json`'s
    /// `meeting_id` at init (replacing the old hardcoded `None`). If the folder/metadata
    /// already exist (id set after init, or resume), the change is persisted immediately.
    pub fn set_meeting_id(&mut self, id: Option<String>) {
        self.meeting_id = id.clone();
        if let Some(ref mut metadata) = self.metadata {
            metadata.meeting_id = id;
            if let Some(folder) = &self.meeting_folder {
                let metadata_clone = metadata.clone();
                if let Err(e) = self.write_metadata(folder, &metadata_clone) {
                    warn!("Failed to update metadata with meeting_id: {}", e);
                }
            }
        }
    }

    /// The DB `meeting_id` this recording is bound to (set at start, written into the
    /// folder metadata). This is the AUTHORITATIVE save target — the frontend's mutable
    /// "current meeting" selection can drift if the user browses another meeting while
    /// recording (v1.6.1 fix), so the stop path saves transcripts to THIS id.
    pub fn get_meeting_id(&self) -> Option<String> {
        self.meeting_id.clone()
    }

    /// Enable RESUME mode (specs/0037): the next `start_accumulation` reuses
    /// `resume_folder` (no fresh folder), reads its `metadata.json`, allocates the next
    /// segment index, and appends without clobbering prior sessions' audio. Also sets the
    /// meeting id so the resumed session stays linked to the same DB row.
    pub fn set_resume_context(&mut self, meeting_id: String, resume_folder: PathBuf) {
        self.meeting_id = Some(meeting_id);
        self.resume_folder = Some(resume_folder);
    }

    /// Audio offset (seconds) for THIS session: the sum of existing segments'
    /// `duration_seconds` in resume mode, or `0.0` for a fresh recording. The stop path
    /// hands this to the DB as the append offset for the new transcript rows. Valid once
    /// `start_accumulation` has run (before that it is `0.0`).
    pub fn prior_audio_duration_seconds(&self) -> f64 {
        self.prior_audio_duration
    }

    /// Set device information in metadata
    pub fn set_device_info(&mut self, mic_name: Option<String>, sys_name: Option<String>) {
        if let Some(ref mut metadata) = self.metadata {
            metadata.devices.microphone = mic_name;
            metadata.devices.system_audio = sys_name;

            // Write updated metadata to disk if folder exists
            if let Some(folder) = &self.meeting_folder {
                let metadata_clone = metadata.clone();
                if let Err(e) = self.write_metadata(folder, &metadata_clone) {
                    warn!("Failed to update metadata with device info: {}", e);
                }
            }
        }
    }

    /// Add or update a structured transcript segment (upserts based on sequence_id)
    /// Also saves incrementally to disk
    pub fn add_transcript_segment(&self, segment: TranscriptSegment) {
        if let Ok(mut segments) = self.transcript_segments.lock() {
            // Check if segment with same sequence_id exists (update it)
            if let Some(existing) = segments
                .iter_mut()
                .find(|s| s.sequence_id == segment.sequence_id)
            {
                *existing = segment.clone();
                info!(
                    "Updated transcript segment {} (seq: {}) - total segments: {}",
                    segment.id,
                    segment.sequence_id,
                    segments.len()
                );
            } else {
                // New segment, add it
                segments.push(segment.clone());
                info!(
                    "Added new transcript segment {} (seq: {}) - total segments: {}",
                    segment.id,
                    segment.sequence_id,
                    segments.len()
                );
            }
        } else {
            error!(
                "Failed to lock transcript segments for adding segment {}",
                segment.id
            );
        }

        // NEW: Save incrementally to disk
        if let Some(folder) = &self.meeting_folder {
            if let Err(e) = self.write_transcripts_json(folder) {
                warn!("Failed to write incremental transcript update: {}", e);
            }
        }
    }

    /// Legacy method for backward compatibility - converts text to basic segment
    pub fn add_transcript_chunk(&self, text: String) {
        let segment = TranscriptSegment {
            id: format!("seg_{}", chrono::Utc::now().timestamp_millis()),
            text,
            audio_start_time: 0.0,
            audio_end_time: 0.0,
            duration: 0.0,
            display_time: "[00:00]".to_string(),
            confidence: 1.0,
            sequence_id: 0,
            channel: None,
        };
        self.add_transcript_segment(segment);
    }

    /// Start accumulation with optional incremental saving
    ///
    /// # Arguments
    /// * `auto_save` - If true, creates checkpoints and enables saving. If false, audio chunks are discarded.
    ///
    /// # Errors
    /// In RESUME mode (specs/0037) a folder-init failure (missing folder, unreadable
    /// metadata, checkpoint dir/saver failure) is FATAL and returned as `Err` — the
    /// session must not appear live while recording nothing into the meeting it claims
    /// to resume. Fresh-session init stays best-effort (log-and-continue), unchanged.
    pub fn start_accumulation(
        &mut self,
        auto_save: bool,
    ) -> Result<mpsc::UnboundedSender<AudioChunk>> {
        if auto_save {
            info!("Initializing incremental audio saver for recording (auto-save ENABLED)");
        } else {
            info!(
                "Starting recording without audio saving (auto-save DISABLED - transcripts only)"
            );
        }

        // Create channel for receiving audio chunks
        let (sender, receiver) = mpsc::unbounded_channel::<AudioChunk>();
        self.chunk_receiver = Some(receiver);

        // Initialize (or, in resume mode, reuse) the meeting folder. `.checkpoints/` +
        // IncrementalAudioSaver are only created when auto_save is enabled; otherwise the
        // folder still holds transcripts/metadata. Resume mode reuses an existing folder
        // regardless of whether a meeting_name was set (it is read back from metadata).
        if self.resume_folder.is_some() {
            match self.initialize_meeting_folder(self.meeting_name.clone().as_deref(), auto_save) {
                Ok(()) => info!("Successfully reused meeting folder for resumed session"),
                Err(e) => {
                    error!("Failed to reuse meeting folder for resume: {}", e);
                    // FATAL for a resume: propagate so the start fails instead of
                    // recording a session that writes nothing to the meeting folder.
                    return Err(e.context("failed to initialize resumed meeting folder"));
                }
            }
        } else if let Some(name) = self.meeting_name.clone() {
            match self.initialize_meeting_folder(Some(&name), auto_save) {
                Ok(()) => info!(
                    "Successfully initialized meeting folder (checkpoints: {})",
                    auto_save
                ),
                Err(e) => {
                    error!("Failed to initialize meeting folder: {}", e);
                    // Continue anyway - will use fallback flat structure
                }
            }
        }

        // Start accumulation task
        let is_saving_clone = self.is_saving.clone();
        let incremental_saver_arc = self.incremental_saver.clone();
        let save_audio = auto_save;

        if let Some(mut receiver) = self.chunk_receiver.take() {
            tokio::spawn(async move {
                info!(
                    "Recording saver accumulation task started (save_audio: {})",
                    save_audio
                );

                while let Some(chunk) = receiver.recv().await {
                    // Check if we should continue
                    let should_continue = if let Ok(is_saving) = is_saving_clone.lock() {
                        *is_saving
                    } else {
                        false
                    };

                    if !should_continue {
                        break;
                    }

                    // Only process audio chunks if auto_save is enabled
                    if save_audio {
                        // Add chunk to incremental saver
                        if let Some(saver_arc) = &incremental_saver_arc {
                            let mut saver_guard = saver_arc.lock().await;
                            if let Err(e) = saver_guard.add_chunk(chunk) {
                                error!("Failed to add chunk to incremental saver: {}", e);
                            }
                        } else {
                            error!("Incremental saver not available while accumulating");
                        }
                    } else {
                        // auto_save is false: discard audio chunk (no-op)
                        // Transcription already happened in the pipeline before this point
                    }
                }

                info!("Recording saver accumulation task ended");
            });
        }

        // Set saving flag
        if let Ok(mut is_saving) = self.is_saving.lock() {
            *is_saving = true;
        }

        Ok(sender)
    }

    /// Confirm the recording start succeeded end-to-end (streams + pipeline live):
    /// drop the resume-init rollback journal so nothing can later unwind a session
    /// that is actually recording. Call once per successful start; no-op otherwise.
    pub fn commit_start(&mut self) {
        self.resume_init_journal = None;
    }

    /// Unwind a start that failed AFTER `start_accumulation` succeeded (specs/0037):
    /// stop the accumulation task, delete any checkpoint chunks THIS session already
    /// wrote (prefix-scoped — never a prior session's preserved chunks), and, for a
    /// resume, replay the init journal so the folder is left exactly as found — a
    /// cleanly-completed meeting keeps `status: "completed"` + canonical
    /// `audio.mp4`/`system.wav`/`mic.wav` (no phantom "Unfinished recording" offer,
    /// diarization/retranscription resolution intact), while a genuinely crashed
    /// meeting stays offerable for the next resume. For a FRESH (non-resume) session
    /// there is no journal — `initialize_resume_folder` is the only path that arms one
    /// — so a failed fresh start instead removes the whole folder `initialize_fresh_folder`
    /// just created: it holds nothing worth keeping (no prior segments, nothing to
    /// resume into), and leaving it behind orphans a meeting-less folder on disk
    /// (specs/0060 review finding — hit in practice by a screenshot-driver take whose
    /// `start_recording` failed after folder init). Best-effort; never fails.
    pub async fn rollback_failed_start(&mut self) {
        let _lease = self.folder_lease.take(); // released once the folder is unwound
        if let Ok(mut is_saving) = self.is_saving.lock() {
            *is_saving = false;
        }

        if let Some(saver_arc) = self.incremental_saver.take() {
            let saver = saver_arc.lock().await;
            saver.cleanup_own_chunks();
        }

        if let Some(journal) = self.resume_init_journal.take() {
            warn!(
                "Recording start failed after resume init — rolling back folder {}",
                journal.folder.display()
            );
            journal.rollback();
            self.meeting_folder = None;
            self.metadata = None;
            self.prior_transcript_segments.clear();
            self.segment_started_at = None;
            self.prior_audio_duration = 0.0;
            self.segment_index = 0;
        } else if let Some(folder) = self.meeting_folder.take() {
            warn!(
                "Recording start failed after fresh folder init — removing {}",
                folder.display()
            );
            if let Err(e) = std::fs::remove_dir_all(&folder) {
                warn!(
                    "Failed to remove folder {} after failed start: {}",
                    folder.display(),
                    e
                );
            }
            self.metadata = None;
        }
    }

    /// Initialize (fresh) or reuse (resume) the meeting folder + metadata for this
    /// session. Dispatches to [`initialize_resume_folder`](Self::initialize_resume_folder)
    /// when `set_resume_context` was called, else [`initialize_fresh_folder`](Self::initialize_fresh_folder).
    ///
    /// # Arguments
    /// * `meeting_name` - Name of the meeting (required for a fresh recording; optional in
    ///   resume mode, where it is read back from the existing metadata if absent).
    /// * `create_checkpoints` - Whether to create `.checkpoints/` + IncrementalAudioSaver.
    fn initialize_meeting_folder(
        &mut self,
        meeting_name: Option<&str>,
        create_checkpoints: bool,
    ) -> Result<()> {
        if let Some(resume_folder) = self.resume_folder.clone() {
            self.initialize_resume_folder(resume_folder, create_checkpoints)
        } else {
            let name = meeting_name
                .ok_or_else(|| anyhow::anyhow!("meeting_name required for a fresh recording"))?;
            self.initialize_fresh_folder(name, create_checkpoints)
        }
    }

    /// Fresh-recording folder init (session #0). Behaviour is identical to pre-0037 apart
    /// from now writing `meeting_id` into metadata (previously hardcoded `None`) and an
    /// empty `segments` list; the audio files themselves are byte-identical.
    fn initialize_fresh_folder(
        &mut self,
        meeting_name: &str,
        create_checkpoints: bool,
    ) -> Result<()> {
        // The active write root: the persisted `save_folder` (specs/0057 Plan 2).
        let base_folder = crate::audio::recordings_root();

        // Create meeting folder structure (with or without .checkpoints/ subdirectory)
        let meeting_folder = create_meeting_folder(&base_folder, meeting_name, create_checkpoints)?;

        // Only initialize incremental saver if checkpoints are needed (auto_save is true)
        if create_checkpoints {
            let incremental_saver = IncrementalAudioSaver::new(meeting_folder.clone(), 48000)?;
            self.incremental_saver = Some(Arc::new(AsyncMutex::new(incremental_saver)));
            info!(
                "✅ Incremental audio saver initialized for meeting: {}",
                meeting_name
            );
        } else {
            info!("⚠️  Skipped incremental audio saver (auto-save disabled)");
        }

        let now = chrono::Utc::now().to_rfc3339();

        // Create initial metadata
        let metadata = MeetingMetadata {
            version: "1.0".to_string(),
            meeting_id: self.meeting_id.clone(), // specs/0037: link folder → DB row at start
            meeting_name: Some(meeting_name.to_string()),
            created_at: now.clone(),
            completed_at: None,
            duration_seconds: None,
            devices: DeviceInfo {
                microphone: None, // Could be enhanced to store actual device names
                system_audio: None,
            },
            audio_file: if create_checkpoints {
                MEETING_AUDIO_FILENAME.to_string()
            } else {
                "".to_string()
            },
            transcript_file: "transcripts.json".to_string(),
            sample_rate: 48000,
            status: "recording".to_string(),
            segments: Vec::new(),
        };

        // Write initial metadata.json
        self.write_metadata(&meeting_folder, &metadata)?;

        self.segment_index = 0;
        self.segment_started_at = Some(now);
        self.prior_audio_duration = 0.0;
        self.meeting_folder = Some(meeting_folder);
        self.metadata = Some(metadata);

        Ok(())
    }

    /// Resume-mode folder init (specs/0037): reuse an existing meeting folder, read its
    /// metadata, migrate prior sessions to segment-scoped filenames (recovering any
    /// crashed session's loose checkpoint chunks) so this session cannot clobber them,
    /// load the existing transcripts so the first incremental write can't destroy them,
    /// allocate the next FREE segment index, and prepare a segment-scoped
    /// IncrementalAudioSaver. `status` stays `"recording"`.
    ///
    /// All destructive on-disk mutations are journaled into `resume_init_journal`; on
    /// any error here the journal is rolled back before returning, and it stays armed
    /// afterwards so [`rollback_failed_start`](Self::rollback_failed_start) can unwind
    /// a start that fails later (streams/pipeline). Errors PROPAGATE — a failed resume
    /// init must fail the start (see `start_accumulation`).
    fn initialize_resume_folder(
        &mut self,
        meeting_folder: PathBuf,
        create_checkpoints: bool,
    ) -> Result<()> {
        if !meeting_folder.exists() {
            return Err(anyhow::anyhow!(
                "resume folder does not exist: {}",
                meeting_folder.display()
            ));
        }

        // Snapshot the ORIGINAL metadata bytes before anything mutates the folder, so a
        // failed start can restore them verbatim.
        let metadata_path = meeting_folder.join("metadata.json");
        let original_metadata_json = std::fs::read_to_string(&metadata_path)
            .map_err(|e| anyhow::anyhow!("failed to read {}: {}", metadata_path.display(), e))?;
        let mut metadata: MeetingMetadata = serde_json::from_str(&original_metadata_json)
            .map_err(|e| anyhow::anyhow!("failed to parse {}: {}", metadata_path.display(), e))?;

        let mut journal = ResumeInitJournal {
            folder: meeting_folder.clone(),
            original_metadata_json,
            renames: Vec::new(),
            created_checkpoints_dir: false,
        };

        // Preserve the crashed/prior session's transcripts: the resumed saver starts
        // with an empty live vec pointed at the SAME folder, so without this the first
        // `add_transcript_segment` would atomically overwrite transcripts.json — the
        // prior session's only on-disk transcript copy.
        self.prior_transcript_segments = Self::read_prior_transcript_segments(&meeting_folder);
        if !self.prior_transcript_segments.is_empty() {
            info!(
                "Resume: preserving {} prior transcript segment(s) from transcripts.json",
                self.prior_transcript_segments.len()
            );
        }

        // Ensure every prior session points at segment-scoped files and recover any
        // crashed session's surviving checkpoint chunks. Idempotent; renames are
        // journaled for rollback.
        Self::migrate_prior_segments(&meeting_folder, &mut metadata, &mut journal.renames);

        // Offset for this session = sum of all existing segments' durations.
        self.prior_audio_duration = metadata
            .segments
            .iter()
            .filter_map(|s| s.duration_seconds)
            .sum();

        // Next FREE index: past both the segment count and every allocated index —
        // `segments.len()` alone would REUSE the index of a crashed resumed session
        // (whose segment is only pushed at stop), overwriting its recovered audio.
        let next_index = Self::next_segment_index(&metadata);

        if create_checkpoints {
            // The prior session deletes `.checkpoints/` at finalize, but recreate it
            // defensively (crash-recovery folders may still hold one), then use a
            // segment-scoped saver so nothing collides with any surviving chunks.
            let checkpoints_dir = meeting_folder.join(".checkpoints");
            if !checkpoints_dir.exists() {
                if let Err(e) = std::fs::create_dir_all(&checkpoints_dir) {
                    self.prior_transcript_segments.clear();
                    journal.rollback();
                    return Err(anyhow::anyhow!(
                        "failed to create {}: {}",
                        checkpoints_dir.display(),
                        e
                    ));
                }
                journal.created_checkpoints_dir = true;
            }
            match IncrementalAudioSaver::new_with_segment(meeting_folder.clone(), 48000, next_index)
            {
                Ok(incremental_saver) => {
                    self.incremental_saver = Some(Arc::new(AsyncMutex::new(incremental_saver)));
                    info!(
                        "✅ Resumed incremental audio saver (segment {}) for folder: {}",
                        next_index,
                        meeting_folder.display()
                    );
                }
                Err(e) => {
                    self.prior_transcript_segments.clear();
                    journal.rollback();
                    return Err(e);
                }
            }
        } else {
            info!("⚠️  Skipped incremental audio saver on resume (auto-save disabled)");
        }

        // Re-link the DB row + keep recording status; adopt existing name if none was set.
        if let Some(id) = &self.meeting_id {
            metadata.meeting_id = Some(id.clone());
        }
        if self.meeting_name.is_none() {
            self.meeting_name = metadata.meeting_name.clone();
        }
        metadata.status = "recording".to_string();
        metadata.audio_file = MEETING_AUDIO_FILENAME.to_string();
        if let Err(e) = self.write_metadata(&meeting_folder, &metadata) {
            self.incremental_saver = None;
            self.prior_transcript_segments.clear();
            journal.rollback();
            return Err(e);
        }

        self.segment_index = next_index;
        self.segment_started_at = Some(chrono::Utc::now().to_rfc3339());
        self.meeting_folder = Some(meeting_folder);
        self.metadata = Some(metadata);
        // Keep the journal armed: a later start failure (pipeline/streams) rolls it
        // back via `rollback_failed_start`; a successful start drops it (`commit_start`).
        self.resume_init_journal = Some(journal);

        Ok(())
    }

    /// The next free segment index for a resumed session: past both the segment COUNT
    /// and the highest allocated INDEX (recovered crashed segments can leave gaps or
    /// exceed the count — reusing their index would overwrite their audio).
    fn next_segment_index(metadata: &MeetingMetadata) -> u32 {
        metadata
            .segments
            .iter()
            .map(|s| s.index + 1)
            .max()
            .unwrap_or(0)
            .max(metadata.segments.len() as u32)
    }

    /// Load the segments of an existing `transcripts.json` (best-effort). Missing file →
    /// empty. An unparseable file is preserved as `transcripts.json.corrupt` (instead of
    /// being silently overwritten by the session's first incremental write) → empty.
    fn read_prior_transcript_segments(folder: &Path) -> Vec<TranscriptSegment> {
        #[derive(Deserialize)]
        struct TranscriptsFile {
            #[serde(default)]
            segments: Vec<TranscriptSegment>,
        }

        let path = folder.join("transcripts.json");
        if !path.exists() {
            return Vec::new();
        }
        let parsed = std::fs::read_to_string(&path)
            .map_err(anyhow::Error::from)
            .and_then(|s| serde_json::from_str::<TranscriptsFile>(&s).map_err(Into::into));
        match parsed {
            Ok(file) => file.segments,
            Err(e) => {
                warn!(
                    "Resume: could not parse existing {} ({}); preserving it as .corrupt",
                    path.display(),
                    e
                );
                let _ = std::fs::rename(&path, folder.join("transcripts.json.corrupt"));
                Vec::new()
            }
        }
    }

    /// Ensure every existing segment references segment-scoped files, renaming the plain
    /// `audio.mp4`/`system.wav`/`mic.wav` on disk when needed (best-effort; missing files
    /// are skipped). If a folder predates the `segments` field entirely, synthesize
    /// `segments[0]` from the folder's top-level metadata first. Every rename performed
    /// is recorded into `rename_journal` so a failed resume start can undo it.
    ///
    /// Also recovers ANY **crash-interrupted** session — first OR resumed (specs/0037):
    /// a session that was force-quit mid-capture never ran `finalize()`, so its audio
    /// survives ONLY as loose checkpoint chunks (`.checkpoints/audio_chunk_*.mp4` for
    /// session 0, `seg{NN}_audio_chunk_*.mp4` for a resumed session). For each chunk
    /// group whose segment index is NOT yet in `metadata.segments`, a `RecordingSegment`
    /// is synthesized; then every segment whose audio file is missing but whose chunks
    /// survive gets them merged into its `audio_seg{NN}.mp4` (chunks are consumed on
    /// success, PRESERVED on failure so a later resume can retry) and a real (probed) or
    /// estimated duration, so the transcript-append offset is never zero. Orphan
    /// `audio_seg{NN}.mp4` files not referenced by any segment (a crash — or rolled-back
    /// start — between a recovery merge and the metadata write) are adopted the same way.
    /// The synthesized indices also push `next_segment_index` past them, so a new resume
    /// can never reuse a crashed session's index and overwrite its chunks.
    fn migrate_prior_segments(
        folder: &Path,
        metadata: &mut MeetingMetadata,
        rename_journal: &mut Vec<(PathBuf, PathBuf)>,
    ) {
        Self::migrate_prior_segments_with(
            folder,
            metadata,
            rename_journal,
            &ffmpeg_concat,
            &Self::probe_audio_duration,
        );
    }

    /// Implementation of [`migrate_prior_segments`](Self::migrate_prior_segments) with the
    /// checkpoint merge + duration probe injected, so the crash-recovery file bookkeeping
    /// can be unit-tested without invoking real ffmpeg on synthetic bytes.
    fn migrate_prior_segments_with(
        folder: &Path,
        metadata: &mut MeetingMetadata,
        rename_journal: &mut Vec<(PathBuf, PathBuf)>,
        concat: &dyn Fn(&[PathBuf], &Path) -> Result<()>,
        probe_duration: &dyn Fn(&Path) -> Option<f64>,
    ) {
        // 1. A pre-0037 folder (no `segments`) gets segment 0 synthesized from the
        //    top-level metadata.
        if metadata.segments.is_empty() {
            metadata.segments.push(RecordingSegment {
                index: 0,
                started_at: metadata.created_at.clone(),
                completed_at: metadata.completed_at.clone(),
                duration_seconds: metadata.duration_seconds,
                audio_file: MEETING_AUDIO_FILENAME.to_string(),
                system_wav: SYSTEM_CHANNEL_FILENAME.to_string(),
                mic_wav: MIC_CHANNEL_FILENAME.to_string(),
            });
        }

        // 2. Move plain-named files out of the way of the new session (journaled).
        for seg in metadata.segments.iter_mut() {
            if seg.audio_file == MEETING_AUDIO_FILENAME {
                let scoped = format!("audio_seg{:02}.mp4", seg.index);
                Self::rename_journaled(folder, MEETING_AUDIO_FILENAME, &scoped, rename_journal);
                seg.audio_file = scoped;
            }
            // A kept meeting's channels may be compressed (`system.opus`, specs/0072).
            for (name, stem) in [(&mut seg.system_wav, "system"), (&mut seg.mic_wav, "mic")] {
                if *name == format!("{stem}.wav") {
                    let plain = channel_files::plain_channel_name(folder, stem);
                    *name = channel_files::scoped_channel_name(&plain, seg.index);
                    Self::rename_journaled(folder, &plain, name, rename_journal);
                }
            }
        }

        // 3. Synthesize segments for crashed RESUMED sessions: seg-prefixed chunk groups
        //    whose index is unknown to metadata (their segment is only recorded at stop,
        //    which the crash never reached). Without this, the next resume would REUSE
        //    that index and its fresh saver would overwrite the crashed session's chunks.
        let chunk_groups = Self::prefixed_checkpoint_groups(folder);
        let known: std::collections::HashSet<u32> =
            metadata.segments.iter().map(|s| s.index).collect();
        let mut recovered_indices: Vec<u32> = Vec::new();
        for (&idx, chunks) in &chunk_groups {
            if idx == 0 || known.contains(&idx) {
                continue;
            }
            let started_at = chunks
                .first()
                .and_then(|c| Self::file_mtime_rfc3339(c))
                .unwrap_or_else(|| metadata.created_at.clone());
            info!(
                "Resume: found {} orphan checkpoint chunk(s) for crashed segment {} — \
                 synthesizing its RecordingSegment",
                chunks.len(),
                idx
            );
            metadata.segments.push(RecordingSegment {
                index: idx,
                started_at,
                completed_at: None,
                duration_seconds: None,
                audio_file: format!("audio_seg{:02}.mp4", idx),
                system_wav: format!("system_seg{:02}.wav", idx),
                mic_wav: format!("mic_seg{:02}.wav", idx),
            });
            recovered_indices.push(idx);
        }
        // The plain per-channel WAVs on disk (if any) were written by the MOST RECENT
        // session — for a crashed resume that's the highest recovered index (they're
        // only renamed to scoped names at stop, which never ran). Step 2 already
        // handled the crashed-fresh-session case via segment 0.
        if let Some(&latest) = recovered_indices.iter().max() {
            Self::rename_journaled(
                folder,
                SYSTEM_CHANNEL_FILENAME,
                &format!("system_seg{:02}.wav", latest),
                rename_journal,
            );
            Self::rename_journaled(
                folder,
                MIC_CHANNEL_FILENAME,
                &format!("mic_seg{:02}.wav", latest),
                rename_journal,
            );
        }

        // 3b. Adopt orphan `audio_seg{NN}.mp4` files referenced by no segment — left by a
        //     crash (or a rolled-back failed start) between a recovery merge and the
        //     metadata write. Without adoption their index would be reused and the file
        //     overwritten by the new session's finalize.
        let known: std::collections::HashSet<u32> =
            metadata.segments.iter().map(|s| s.index).collect();
        for (idx, path) in Self::orphan_segment_audio_files(folder) {
            if known.contains(&idx) {
                continue;
            }
            info!(
                "Resume: adopting orphan segment audio {} (no metadata entry)",
                path.display()
            );
            metadata.segments.push(RecordingSegment {
                index: idx,
                started_at: Self::file_mtime_rfc3339(&path)
                    .unwrap_or_else(|| metadata.created_at.clone()),
                completed_at: None,
                duration_seconds: probe_duration(&path).filter(|d| *d > 0.0),
                audio_file: format!("audio_seg{:02}.mp4", idx),
                system_wav: format!("system_seg{:02}.wav", idx),
                mic_wav: format!("mic_seg{:02}.wav", idx),
            });
        }

        // 4. Crash recovery proper: for every segment whose audio is missing but whose
        //    checkpoint chunks survive, merge the chunks into the segment file. A
        //    cleanly-stopped segment (real audio + duration) skips this untouched.
        for seg in metadata.segments.iter_mut() {
            let seg_audio = folder.join(&seg.audio_file);
            let chunks: Vec<PathBuf> = if seg.index == 0 {
                Self::session_zero_checkpoint_chunks(folder)
            } else {
                chunk_groups.get(&seg.index).cloned().unwrap_or_default()
            };

            if !seg_audio.exists() {
                if chunks.is_empty() {
                    warn!(
                        "Resume: segment {} audio {} is missing and no checkpoint chunks \
                         survive — pre-resume audio for this segment is unrecoverable",
                        seg.index, seg.audio_file
                    );
                    continue;
                }
                match concat(&chunks, &seg_audio) {
                    Ok(()) => {
                        // Consume the merged chunks so the new session's `finalize()`
                        // can't delete still-unmerged pre-crash audio, and a re-resume
                        // won't double-count them.
                        for c in &chunks {
                            let _ = std::fs::remove_file(c);
                        }
                        // Prefer the merged file's real (probed) duration; fall back to
                        // a coarse chunk-count estimate so the append offset is always
                        // non-zero (else resumed transcripts overlap at t=0).
                        let estimated = chunks.len() as f64 * CHECKPOINT_SECONDS;
                        let dur = probe_duration(&seg_audio)
                            .filter(|d| *d > 0.0)
                            .unwrap_or(estimated);
                        seg.duration_seconds = Some(dur);
                        info!(
                            "Recovered crashed segment {}: merged {} checkpoint \
                             chunk(s) → {} ({:.2}s)",
                            seg.index,
                            chunks.len(),
                            seg.audio_file,
                            dur
                        );
                    }
                    Err(e) => {
                        error!(
                            "Failed to merge crashed segment {} checkpoints into {}: {}",
                            seg.index, seg.audio_file, e
                        );
                        // Merge failed but the chunks are PRESERVED on disk (they're the
                        // only copy; the new session's prefix-scoped finalize won't touch
                        // them, and a later resume retries this merge). Still give the
                        // segment a clearly non-None (approximate) duration so the
                        // transcript append offset isn't zero.
                        if seg.duration_seconds.is_none_or(|d| d <= 0.0) {
                            seg.duration_seconds = Some(chunks.len() as f64 * CHECKPOINT_SECONDS);
                        }
                    }
                }
            } else if seg.duration_seconds.is_none_or(|d| d <= 0.0) {
                // Audio exists but the duration was never recorded (e.g. a recovery
                // merge whose metadata write was rolled back by a failed start):
                // re-probe so the append offset self-heals.
                if let Some(dur) = probe_duration(&seg_audio).filter(|d| *d > 0.0) {
                    seg.duration_seconds = Some(dur);
                }
            }
        }

        metadata.segments.sort_by_key(|s| s.index);
    }

    /// List the first session's surviving checkpoint chunks
    /// (`.checkpoints/audio_chunk_NNN.mp4`), sorted by filename. Resumed sessions prefix
    /// their chunks (`segNN_audio_chunk_*.mp4`), so the `audio_chunk_` name filter selects
    /// ONLY session #0's — the ones stranded by a crash before finalize (specs/0037).
    fn session_zero_checkpoint_chunks(folder: &Path) -> Vec<PathBuf> {
        let dir = folder.join(".checkpoints");
        let mut chunks: Vec<PathBuf> = match std::fs::read_dir(&dir) {
            Ok(entries) => entries
                .filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| {
                    p.extension().and_then(|s| s.to_str()) == Some("mp4")
                        && p.file_name()
                            .and_then(|n| n.to_str())
                            .map(|n| n.starts_with("audio_chunk_"))
                            .unwrap_or(false)
                })
                .collect(),
            Err(_) => return Vec::new(),
        };
        chunks.sort();
        chunks
    }

    /// Group the RESUMED sessions' surviving checkpoint chunks
    /// (`.checkpoints/seg{NN}_audio_chunk_MMM.mp4`) by segment index, each group sorted
    /// by filename. These are the chunks stranded when a resumed session crashed before
    /// its finalize (specs/0037).
    fn prefixed_checkpoint_groups(folder: &Path) -> std::collections::BTreeMap<u32, Vec<PathBuf>> {
        let dir = folder.join(".checkpoints");
        let mut groups: std::collections::BTreeMap<u32, Vec<PathBuf>> = Default::default();
        let Ok(entries) = std::fs::read_dir(&dir) else {
            return groups;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("mp4") {
                continue;
            }
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            // seg{NN}_audio_chunk_{MMM}.mp4
            let Some(rest) = name.strip_prefix("seg") else {
                continue;
            };
            let Some((index_str, tail)) = rest.split_once('_') else {
                continue;
            };
            if !tail.starts_with("audio_chunk_") {
                continue;
            }
            if let Ok(idx) = index_str.parse::<u32>() {
                groups.entry(idx).or_default().push(path);
            }
        }
        for chunks in groups.values_mut() {
            chunks.sort();
        }
        groups
    }

    /// List `audio_seg{NN}.mp4` files in the meeting folder, keyed by segment index.
    /// Used to adopt merged segment audio that lost its metadata entry (crash or
    /// rolled-back start between the recovery merge and the metadata write).
    fn orphan_segment_audio_files(folder: &Path) -> std::collections::BTreeMap<u32, PathBuf> {
        let mut found: std::collections::BTreeMap<u32, PathBuf> = Default::default();
        let Ok(entries) = std::fs::read_dir(folder) else {
            return found;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if let Some(idx) = name
                .strip_prefix("audio_seg")
                .and_then(|s| s.strip_suffix(".mp4"))
                .and_then(|s| s.parse::<u32>().ok())
            {
                found.insert(idx, path);
            }
        }
        found
    }

    /// A file's modification time as RFC3339 (best-effort) — the closest available
    /// approximation of a crashed session's timeline for synthesized segments.
    fn file_mtime_rfc3339(path: &Path) -> Option<String> {
        std::fs::metadata(path)
            .and_then(|m| m.modified())
            .ok()
            .map(|t| chrono::DateTime::<chrono::Utc>::from(t).to_rfc3339())
    }

    /// Probe an audio file's duration (seconds) via the shared symphonia metadata probe
    /// (`import::extract_duration_from_metadata`). Returns `None` for a missing/unreadable
    /// file (e.g. a synthetic test fixture) so callers fall back to an approximation.
    fn probe_audio_duration(path: &Path) -> Option<f64> {
        super::import::extract_duration_from_metadata(path)
            .ok()
            .filter(|d| *d > 0.0)
    }

    /// Rename `folder/from` → `folder/to` if the source exists (best-effort; logs on error).
    fn rename_if_exists(folder: &Path, from: &str, to: &str) {
        let mut discard = Vec::new();
        Self::rename_journaled(folder, from, to, &mut discard);
    }

    /// [`rename_if_exists`](Self::rename_if_exists) that records a performed rename as
    /// `(original, renamed_to)` into `journal`, so a failed resume start can undo it.
    fn rename_journaled(
        folder: &Path,
        from: &str,
        to: &str,
        journal: &mut Vec<(PathBuf, PathBuf)>,
    ) {
        let src = folder.join(from);
        if src.exists() {
            let dst = folder.join(to);
            match std::fs::rename(&src, &dst) {
                Ok(()) => journal.push((src, dst)),
                Err(e) => warn!(
                    "Failed to rename {} → {} for resume: {}",
                    src.display(),
                    dst.display(),
                    e
                ),
            }
        }
    }

    /// Write metadata.json to disk (atomic write with temp file)
    fn write_metadata(&self, folder: &Path, metadata: &MeetingMetadata) -> Result<()> {
        let metadata_path = folder.join("metadata.json");
        let temp_path = folder.join(".metadata.json.tmp");

        let json_string = serde_json::to_string_pretty(metadata)?;
        std::fs::write(&temp_path, json_string)?;
        std::fs::rename(&temp_path, &metadata_path)?; // Atomic

        Ok(())
    }

    /// Write transcripts.json to disk (atomic write with temp file and validation).
    ///
    /// In resume mode (specs/0037) the prior sessions' segments (loaded at init) are
    /// written FIRST, followed by the current session's — so the atomic overwrite can
    /// never destroy the crashed/prior session's only on-disk transcript copy.
    fn write_transcripts_json(&self, folder: &Path) -> Result<()> {
        // Clone segments to avoid holding lock during I/O
        let current_segments = if let Ok(segments) = self.transcript_segments.lock() {
            segments.clone()
        } else {
            error!("Failed to lock transcript segments for writing");
            return Err(anyhow::anyhow!("Failed to lock transcript segments"));
        };

        // Prior sessions first, current session after (chronological order on disk).
        let mut segments_clone = self.prior_transcript_segments.clone();
        segments_clone.extend(current_segments);

        info!(
            "Writing {} transcript segments to JSON ({} preserved from prior sessions)",
            segments_clone.len(),
            self.prior_transcript_segments.len()
        );

        let transcript_path = folder.join("transcripts.json");
        let temp_path = folder.join(".transcripts.json.tmp");

        // Create JSON structure
        let json = serde_json::json!({
            "version": "1.0",
            "segments": segments_clone,
            "last_updated": chrono::Utc::now().to_rfc3339(),
            "total_segments": segments_clone.len()
        });

        // Serialize to pretty JSON string
        let json_string = serde_json::to_string_pretty(&json).map_err(|e| {
            error!("Failed to serialize transcripts to JSON: {}", e);
            anyhow::anyhow!("JSON serialization failed: {}", e)
        })?;

        // Write to temp file with error handling
        std::fs::write(&temp_path, &json_string).map_err(|e| {
            error!(
                "Failed to write transcript temp file to {}: {}",
                temp_path.display(),
                e
            );
            anyhow::anyhow!("Failed to write temp file: {}", e)
        })?;

        // Verify temp file was written correctly
        if !temp_path.exists() {
            error!(
                "Temp transcript file does not exist after write: {}",
                temp_path.display()
            );
            return Err(anyhow::anyhow!("Temp file verification failed"));
        }

        // Atomic rename
        std::fs::rename(&temp_path, &transcript_path).map_err(|e| {
            error!(
                "Failed to rename transcript file from {} to {}: {}",
                temp_path.display(),
                transcript_path.display(),
                e
            );
            anyhow::anyhow!("Failed to rename transcript file: {}", e)
        })?;

        info!(
            "✅ Successfully wrote transcripts.json with {} segments",
            segments_clone.len()
        );
        Ok(())
    }

    // in frontend/src-tauri/src/audio/recording_saver.rs
    pub fn get_stats(&self) -> (usize, u32) {
        if let Some(ref saver) = self.incremental_saver {
            if let Ok(guard) = saver.try_lock() {
                (guard.get_checkpoint_count() as usize, 48000)
            } else {
                (0, 48000)
            }
        } else {
            (0, 48000)
        }
    }

    /// Stop and save using incremental saving approach
    ///
    /// # Arguments
    /// * `app` - Tauri app handle for emitting events
    /// * `recording_duration` - Actual recording duration in seconds (from RecordingState)
    pub async fn stop_and_save<R: Runtime>(
        &mut self,
        app: &AppHandle<R>,
        recording_duration: Option<f64>,
    ) -> Result<Option<String>, String> {
        info!("Stopping recording saver");
        let _lease = self.folder_lease.take(); // held until finalization returns

        // Stop accumulation
        if let Ok(mut is_saving) = self.is_saving.lock() {
            *is_saving = false;
        }

        // Give time for final chunks
        tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;

        // Check if incremental saver exists (indicates auto_save was enabled)
        let should_save_audio = self.incremental_saver.is_some();

        if !should_save_audio {
            info!("⚠️  No audio saver initialized (auto-save was disabled) - skipping audio finalization");
            info!("✅ Transcripts and metadata already saved incrementally");
            return Ok(None);
        }

        // Finalize incremental saver (merge checkpoints into final audio.mp4)
        let final_audio_path = if let Some(saver_arc) = &self.incremental_saver {
            let mut saver = saver_arc.lock().await;
            match saver.finalize().await {
                Ok(path) => {
                    info!("✅ Successfully finalized audio: {}", path.display());
                    path
                }
                Err(e) => {
                    error!("❌ Failed to finalize incremental saver: {}", e);
                    return Err(format!("Failed to finalize audio: {}", e));
                }
            }
        } else {
            error!("No incremental saver initialized - cannot save recording");
            return Err("No incremental saver initialized".to_string());
        };

        // Save final transcripts.json with validation
        if let Some(folder) = &self.meeting_folder {
            if let Err(e) = self.write_transcripts_json(folder) {
                error!("❌ Failed to write final transcripts: {}", e);
                return Err(format!("Failed to save transcripts: {}", e));
            }

            // Verify transcripts were written correctly
            let transcript_path = folder.join("transcripts.json");
            if !transcript_path.exists() {
                error!(
                    "❌ Transcript file was not created at: {}",
                    transcript_path.display()
                );
                return Err("Transcript file verification failed".to_string());
            }
            info!(
                "✅ Transcripts saved and verified at: {}",
                transcript_path.display()
            );
        }

        // Duration captured in THIS session (used both as the segment duration and, for a
        // lone session, the whole-meeting duration — unchanged from pre-0037). Falls back
        // to the last transcript segment if RecordingState didn't provide one.
        let this_session_duration = recording_duration.or_else(|| {
            if let Ok(segments) = self.transcript_segments.lock() {
                segments.last().map(|seg| seg.audio_end_time)
            } else {
                None
            }
        });

        // The final, canonical meeting audio path. For a lone session this is exactly
        // `final_audio_path` (byte-identical to pre-0037); with segments it becomes the
        // concat target `audio.mp4`.
        let mut meeting_audio_path = final_audio_path.clone();

        // Append this session's segment and, for a multi-segment meeting, concatenate all
        // segment audio + per-channel WAVs into the canonical files. specs/0037.
        if let (Some(folder), Some(mut metadata)) =
            (self.meeting_folder.clone(), self.metadata.clone())
        {
            let is_resume = self.resume_folder.is_some();
            let idx = self.segment_index;

            // Filenames for THIS session's segment. A lone session #0 keeps the plain
            // names (the files the incremental saver + pipeline already wrote); a resumed
            // session uses segment-scoped names so nothing overwrites prior sessions.
            let (audio_name, system_name, mic_name) = if is_resume {
                (
                    format!("audio_seg{:02}.mp4", idx),
                    format!("system_seg{:02}.wav", idx),
                    format!("mic_seg{:02}.wav", idx),
                )
            } else {
                (
                    MEETING_AUDIO_FILENAME.to_string(),
                    SYSTEM_CHANNEL_FILENAME.to_string(),
                    MIC_CHANNEL_FILENAME.to_string(),
                )
            };

            // On resume the pipeline wrote fresh plain `system.wav`/`mic.wav` for this
            // session; move them to the segment-scoped names before concatenation. (The
            // segment audio is already segment-scoped, written directly by `finalize()`.)
            if is_resume {
                Self::rename_if_exists(&folder, SYSTEM_CHANNEL_FILENAME, &system_name);
                Self::rename_if_exists(&folder, MIC_CHANNEL_FILENAME, &mic_name);
            }

            metadata.segments.push(RecordingSegment {
                index: idx,
                started_at: self
                    .segment_started_at
                    .clone()
                    .unwrap_or_else(|| chrono::Utc::now().to_rfc3339()),
                completed_at: Some(chrono::Utc::now().to_rfc3339()),
                duration_seconds: this_session_duration,
                audio_file: audio_name,
                system_wav: system_name,
                mic_wav: mic_name,
            });

            // Multi-segment meeting: concatenate every segment into one continuous file so
            // playback is end-to-end and the offline diarization pass (single `system.wav`,
            // full recompute) covers the whole meeting.
            if metadata.segments.len() > 1 {
                let audio_inputs: Vec<PathBuf> = metadata
                    .segments
                    .iter()
                    .map(|s| folder.join(&s.audio_file))
                    .collect();

                let audio_out = folder.join(MEETING_AUDIO_FILENAME);
                if let Err(e) = ffmpeg_concat(&audio_inputs, &audio_out) {
                    error!("❌ Failed to concat segment audio: {}", e);
                    return Err(format!("Failed to concat segment audio: {}", e));
                }
                meeting_audio_path = audio_out;

                // Per-channel concat is best-effort (missing segments are skipped) and decodes,
                // since a kept segment may be Opus by now (specs/0072).
                channel_files::concat_segment_channels(&folder, &metadata.segments);
            }

            metadata.status = "completed".to_string();
            metadata.completed_at = Some(chrono::Utc::now().to_rfc3339());
            metadata.audio_file = MEETING_AUDIO_FILENAME.to_string();
            // Cumulative meeting duration = sum of all segment durations (for a lone
            // session this equals `this_session_duration`, matching pre-0037).
            let cumulative: f64 = metadata
                .segments
                .iter()
                .filter_map(|s| s.duration_seconds)
                .sum();
            metadata.duration_seconds = if cumulative > 0.0 {
                Some(cumulative)
            } else {
                this_session_duration
            };

            if let Err(e) = self.write_metadata(&folder, &metadata) {
                error!("❌ Failed to update metadata to completed: {}", e);
                return Err(format!("Failed to update metadata: {}", e));
            }

            info!(
                "✅ Metadata updated: {} segment(s), duration {:?}s",
                metadata.segments.len(),
                metadata.duration_seconds
            );
            self.metadata = Some(metadata);
        }

        // Emit save event with audio and transcript paths
        let save_event = serde_json::json!({
            "audio_file": meeting_audio_path.to_string_lossy(),
            "transcript_file": self.meeting_folder.as_ref()
                .map(|f| f.join("transcripts.json").to_string_lossy().to_string()),
            "meeting_name": self.meeting_name,
            "meeting_folder": self.meeting_folder.as_ref()
                .map(|f| f.to_string_lossy().to_string())
        });

        if let Err(e) = app.emit("recording-saved", &save_event) {
            warn!("Failed to emit recording-saved event: {}", e);
        }

        // Clean up transcript segments
        if let Ok(mut segments) = self.transcript_segments.lock() {
            segments.clear();
        }

        Ok(Some(meeting_audio_path.to_string_lossy().to_string()))
    }

    /// Get the meeting folder path (for passing to backend)
    pub fn get_meeting_folder(&self) -> Option<&PathBuf> {
        self.meeting_folder.as_ref()
    }

    /// Get accumulated transcript segments (for reload sync)
    pub fn get_transcript_segments(&self) -> Vec<TranscriptSegment> {
        if let Ok(segments) = self.transcript_segments.lock() {
            segments.clone()
        } else {
            Vec::new()
        }
    }

    /// Get meeting name (for reload sync)
    pub fn get_meeting_name(&self) -> Option<String> {
        self.meeting_name.clone()
    }
}

impl Default for RecordingSaver {
    fn default() -> Self {
        Self::new()
    }
}

/// Concatenate `inputs` (in order) into `output` using the FFmpeg concat demuxer with
/// stream copy — the same fast, no-re-encode path `IncrementalAudioSaver` uses to merge
/// checkpoints, for the segment `.mp4` audio and checkpoint chunks (channel files, which
/// may be Opus, go through `channel_files::concat_segment_channels`). Inputs that do not
/// exist on disk are skipped (a purged meeting's segments); with nothing left to concat
/// this is a no-op.
fn ffmpeg_concat(inputs: &[PathBuf], output: &Path) -> Result<()> {
    let existing: Vec<&PathBuf> = inputs.iter().filter(|p| p.exists()).collect();
    if existing.is_empty() {
        info!(
            "ffmpeg_concat: no existing inputs for {}, skipping",
            output.display()
        );
        return Ok(());
    }

    // Concat list beside the output (its parent always exists — it's the meeting folder).
    let parent = output
        .parent()
        .ok_or_else(|| anyhow::anyhow!("output path has no parent: {}", output.display()))?;
    let list_file = parent.join(".concat_segments.txt");

    let mut list_content = String::new();
    for path in &existing {
        // Absolute paths for FFmpeg's `-safe 0` mode.
        let abs = path.canonicalize()?;
        // Escape single quotes per the concat demuxer's quoting rules.
        let escaped = abs.display().to_string().replace('\'', "'\\''");
        list_content.push_str(&format!("file '{}'\n", escaped));
    }
    std::fs::write(&list_file, list_content)?;

    let ffmpeg_path = find_ffmpeg_path()
        .ok_or_else(|| anyhow::anyhow!("FFmpeg not found. Cannot concatenate segments."))?;

    let mut command = std::process::Command::new(ffmpeg_path);
    command.args([
        "-f",
        "concat",
        "-safe",
        "0",
        "-i",
        list_file.to_str().unwrap(),
        "-c",
        "copy",
        "-y",
        output.to_str().unwrap(),
    ]);

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        command.creation_flags(CREATE_NO_WINDOW);
    }

    let ffmpeg_output = command.output()?;
    let _ = std::fs::remove_file(&list_file);

    if !ffmpeg_output.status.success() {
        let stderr = String::from_utf8_lossy(&ffmpeg_output.stderr);
        error!("FFmpeg segment concat failed: {}", stderr);
        return Err(anyhow::anyhow!("FFmpeg concat failed: {}", stderr));
    }
    if !output.exists() {
        return Err(anyhow::anyhow!(
            "concat output was not created: {}",
            output.display()
        ));
    }

    info!(
        "✅ Concatenated {} segment file(s) → {}",
        existing.len(),
        output.display()
    );
    Ok(())
}

#[cfg(test)]
mod tests;
