use super::encode::encode_single_audio;
use super::recording_state::AudioChunk;
use anyhow::{anyhow, Result};
use log::{error, info, warn};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use super::ffmpeg::find_ffmpeg_path;

/// Audio data without device type (we only store mixed audio)
#[derive(Clone)]
struct AudioData {
    data: Vec<f32>,
    // sample_rate: u32,
}

/// Incremental audio saver that writes checkpoints every 30 seconds
/// to minimize memory usage and enable crash recovery
pub struct IncrementalAudioSaver {
    checkpoint_buffer: Vec<AudioData>,
    checkpoint_interval_samples: usize, // 30s at 48kHz = 1,440,000 samples
    checkpoint_count: u32,
    checkpoints_dir: PathBuf,
    meeting_folder: PathBuf,
    sample_rate: u32,
    /// Prefix for checkpoint chunk filenames. Empty for a fresh/first session
    /// (`audio_chunk_NNN.mp4`, byte-identical to pre-0037 behaviour); `seg{NN}_` for a
    /// resumed session (specs/0037) so its chunks NEVER collide with any prior session's
    /// surviving `.checkpoints/audio_chunk_*.mp4` (e.g. a crash-recovery folder).
    checkpoint_prefix: String,
    /// Final merged filename inside the meeting folder. `audio.mp4` for a fresh/first
    /// session (unchanged); `audio_seg{NN}.mp4` for a resumed session so `finalize()`
    /// does not clobber session #0's `audio.mp4` — the segments are concatenated into the
    /// canonical `audio.mp4` at final stop (see `RecordingSaver::stop_and_save`).
    output_filename: String,
}

impl IncrementalAudioSaver {
    /// Create a new incremental saver for a fresh / first recording session.
    ///
    /// # Arguments
    /// * `meeting_folder` - Path to the meeting folder (contains .checkpoints/)
    /// * `sample_rate` - Sample rate of audio (typically 48000)
    pub fn new(meeting_folder: PathBuf, sample_rate: u32) -> Result<Self> {
        Self::new_inner(
            meeting_folder,
            sample_rate,
            String::new(),
            "audio.mp4".to_string(),
        )
    }

    /// Create a saver for a RESUMED recording session (specs/0037). Checkpoint chunks are
    /// written as `.checkpoints/seg{NN}_audio_chunk_*.mp4` and merged into
    /// `audio_seg{NN}.mp4`, so nothing overwrites a prior session's audio. The
    /// per-segment files are concatenated into the canonical `audio.mp4` at final stop.
    pub fn new_with_segment(
        meeting_folder: PathBuf,
        sample_rate: u32,
        segment_index: u32,
    ) -> Result<Self> {
        Self::new_inner(
            meeting_folder,
            sample_rate,
            format!("seg{:02}_", segment_index),
            format!("audio_seg{:02}.mp4", segment_index),
        )
    }

    fn new_inner(
        meeting_folder: PathBuf,
        sample_rate: u32,
        checkpoint_prefix: String,
        output_filename: String,
    ) -> Result<Self> {
        let checkpoints_dir = meeting_folder.join(".checkpoints");

        // Verify checkpoints directory exists
        if !checkpoints_dir.exists() {
            return Err(anyhow!(
                "Checkpoints directory does not exist: {}",
                checkpoints_dir.display()
            ));
        }

        Ok(Self {
            checkpoint_buffer: Vec::new(),
            checkpoint_interval_samples: sample_rate as usize * 30, // 30 seconds
            checkpoint_count: 0,
            checkpoints_dir,
            meeting_folder,
            sample_rate,
            checkpoint_prefix,
            output_filename,
        })
    }

    /// Add an audio chunk to the buffer
    /// Automatically saves a checkpoint when buffer reaches 30 seconds
    pub fn add_chunk(&mut self, chunk: AudioChunk) -> Result<()> {
        let audio_data = AudioData {
            data: chunk.data,
            // sample_rate: chunk.sample_rate,
        };

        self.checkpoint_buffer.push(audio_data);

        // Calculate total samples in buffer
        let total_samples: usize = self.checkpoint_buffer.iter().map(|c| c.data.len()).sum();

        // Save checkpoint when buffer reaches threshold (30 seconds)
        if total_samples >= self.checkpoint_interval_samples {
            self.save_checkpoint()?;
            self.checkpoint_buffer.clear();
        }

        Ok(())
    }

    /// Save current buffer as a checkpoint file
    fn save_checkpoint(&mut self) -> Result<()> {
        // Concatenate all chunks in buffer
        let audio_data: Vec<f32> = self
            .checkpoint_buffer
            .iter()
            .flat_map(|c| &c.data)
            .cloned()
            .collect();

        if audio_data.is_empty() {
            warn!("Attempted to save empty checkpoint, skipping");
            return Ok(());
        }

        // Generate checkpoint filename (segment-scoped prefix is empty for a fresh session)
        let checkpoint_path = self.checkpoints_dir.join(format!(
            "{}audio_chunk_{:03}.mp4",
            self.checkpoint_prefix, self.checkpoint_count
        ));

        // Encode and save checkpoint
        encode_single_audio(
            bytemuck::cast_slice(&audio_data),
            self.sample_rate,
            1, // mono
            super::encode::MIX_AAC_BITRATE,
            &checkpoint_path,
        )?;

        let duration_seconds = audio_data.len() as f32 / self.sample_rate as f32;
        self.checkpoint_count += 1;

        info!(
            "Saved checkpoint {}: {:.2}s of audio ({} samples)",
            self.checkpoint_count,
            duration_seconds,
            audio_data.len()
        );

        Ok(())
    }

    /// Finalize the recording: save final checkpoint, merge all checkpoints, cleanup
    ///
    /// Returns the path to the final merged audio.mp4 file
    pub async fn finalize(&mut self) -> Result<PathBuf> {
        info!("Finalizing incremental recording...");

        // Save final buffer if not empty
        if !self.checkpoint_buffer.is_empty() {
            info!(
                "Saving final checkpoint with remaining {} chunks",
                self.checkpoint_buffer.len()
            );
            self.save_checkpoint()?;
            self.checkpoint_buffer.clear();
        }

        if self.checkpoint_count == 0 {
            return Err(anyhow!(
                "No audio checkpoints to merge - recording may have failed"
            ));
        }

        // Merge all checkpoints using FFmpeg concat (segment-scoped output for resumed
        // sessions; plain `audio.mp4` for a fresh/first session).
        let final_audio_path = self.meeting_folder.join(&self.output_filename);
        self.merge_checkpoints(&final_audio_path).await?;

        // Clean up ONLY this session's checkpoint chunks (prefix-scoped) + the concat
        // list. A `remove_dir_all` here would also delete chunks PRESERVED from a
        // crashed prior session whose merge failed at resume time (specs/0037) — their
        // only surviving copy. The directory itself is removed only when empty, so a
        // fresh session still fully cleans up exactly as before.
        info!("Cleaning up {} checkpoint files", self.checkpoint_count);
        self.cleanup_own_chunks();
        self.remove_checkpoints_dir_if_empty();

        info!("Finalized recording: {}", final_audio_path.display());

        Ok(final_audio_path)
    }

    /// Delete this session's own checkpoint chunks (`{prefix}audio_chunk_NNN.mp4` for
    /// `0..checkpoint_count`) and the shared `concat_list.txt`. Never touches chunks
    /// with a different segment prefix — those belong to another (possibly crashed)
    /// session and may be their audio's only copy (specs/0037). Best-effort: logs on
    /// error.
    pub fn cleanup_own_chunks(&self) {
        for i in 0..self.checkpoint_count {
            let chunk = self.checkpoints_dir.join(format!(
                "{}audio_chunk_{:03}.mp4",
                self.checkpoint_prefix, i
            ));
            if let Err(e) = std::fs::remove_file(&chunk) {
                if chunk.exists() {
                    warn!("Failed to delete checkpoint {}: {}", chunk.display(), e);
                }
            }
        }
        let _ = std::fs::remove_file(self.checkpoints_dir.join("concat_list.txt"));
    }

    /// Remove the `.checkpoints/` directory iff it is now empty (i.e. no other
    /// session's preserved chunks remain). Best-effort.
    pub fn remove_checkpoints_dir_if_empty(&self) {
        if let Ok(mut entries) = std::fs::read_dir(&self.checkpoints_dir) {
            if entries.next().is_none() {
                if let Err(e) = std::fs::remove_dir(&self.checkpoints_dir) {
                    warn!(
                        "Failed to remove empty checkpoints dir {}: {}",
                        self.checkpoints_dir.display(),
                        e
                    );
                }
            }
        }
    }

    /// Merge all checkpoint files into final audio.mp4 using FFmpeg concat
    /// Uses concat demuxer for fast merging without re-encoding
    async fn merge_checkpoints(&self, output: &Path) -> Result<()> {
        info!(
            "Merging {} checkpoints into final audio file...",
            self.checkpoint_count
        );

        // Create concat list file for FFmpeg
        let list_file = self.checkpoints_dir.join("concat_list.txt");
        let mut list_content = String::new();

        for i in 0..self.checkpoint_count {
            let checkpoint_path = self.checkpoints_dir.join(format!(
                "{}audio_chunk_{:03}.mp4",
                self.checkpoint_prefix, i
            ));

            // Verify checkpoint exists
            if !checkpoint_path.exists() {
                return Err(anyhow!(
                    "Checkpoint file missing: {}",
                    checkpoint_path.display()
                ));
            }

            // Use absolute path for FFmpeg (required for safe mode)
            let abs_path = checkpoint_path.canonicalize()?;
            list_content.push_str(&format!("file '{}'\n", abs_path.display()));
        }

        std::fs::write(&list_file, list_content)?;

        let ffmpeg_path = find_ffmpeg_path().ok_or_else(|| {
            anyhow!("FFmpeg not found. Please install FFmpeg to finalize recordings.")
        })?;
        info!("Using FFmpeg at: {:?}", ffmpeg_path);

        // Run FFmpeg concat command
        // Using concat demuxer with copy codec for fast merging (no re-encoding)

        let mut command = std::process::Command::new(ffmpeg_path);

        command.args([
            "-f",
            "concat", // Use concat demuxer
            "-safe",
            "0", // Allow absolute paths
            "-i",
            list_file.to_str().unwrap(),
            "-c",
            "copy", // Copy codec - no re-encoding!
            "-y",   // Overwrite output file
            output.to_str().unwrap(),
        ]);

        // Hide console window on Windows to prevent CMD popup during finalization
        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x08000000;
            command.creation_flags(CREATE_NO_WINDOW);
        }

        let ffmpeg_output = command.output()?;

        if !ffmpeg_output.status.success() {
            let stderr = String::from_utf8_lossy(&ffmpeg_output.stderr);
            error!("FFmpeg merge failed: {}", stderr);
            return Err(anyhow!("FFmpeg concat failed: {}", stderr));
        }

        // Verify output file was created
        if !output.exists() {
            return Err(anyhow!(
                "Merged audio file was not created: {}",
                output.display()
            ));
        }

        info!(
            "Successfully merged {} checkpoints → {}",
            self.checkpoint_count,
            output.display()
        );

        Ok(())
    }

    /// Get the meeting folder path
    pub fn get_meeting_folder(&self) -> &PathBuf {
        &self.meeting_folder
    }

    /// Get current checkpoint count
    pub fn get_checkpoint_count(&self) -> u32 {
        self.checkpoint_count
    }
}

/// Audio recovery status for transcript recovery feature
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioRecoveryStatus {
    pub status: String, // "success" | "partial" | "failed" | "none"
    pub chunk_count: u32,
    pub estimated_duration_seconds: f64,
    pub audio_file_path: Option<String>,
    pub message: String,
}

/// List the LEGACY (session-0) checkpoint chunks in a `.checkpoints/` dir: only the
/// UNprefixed `audio_chunk_NNN.mp4` files, sorted by filename. Resumed sessions
/// (specs/0037) write `seg{NN}_`-prefixed chunks; a live or crashed resumed session's
/// chunks must NEVER be swept into a legacy session-0 recovery merge, so they are
/// excluded here.
fn legacy_session_chunk_files(checkpoints_dir: &Path) -> Vec<PathBuf> {
    let mut chunks: Vec<PathBuf> = match std::fs::read_dir(checkpoints_dir) {
        Ok(entries) => entries
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
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

/// Recover audio from checkpoint files
/// This is called by the transcript recovery system to merge audio chunks after a crash.
/// Operates ONLY on the legacy unprefixed session-0 chunks (`audio_chunk_*.mp4`);
/// segment-prefixed chunks from resumed sessions (specs/0037) are left untouched.
#[tauri::command]
pub async fn recover_audio_from_checkpoints(
    meeting_folder: String,
    _sample_rate: u32,
) -> Result<AudioRecoveryStatus, String> {
    info!("Starting audio recovery for folder: {}", meeting_folder);

    let folder_path = PathBuf::from(&meeting_folder);
    let checkpoints_dir = folder_path.join(".checkpoints");

    // Check if checkpoints directory exists
    if !checkpoints_dir.exists() {
        info!(
            "No checkpoints directory found at: {}",
            checkpoints_dir.display()
        );
        return Ok(AudioRecoveryStatus {
            status: "none".to_string(),
            chunk_count: 0,
            estimated_duration_seconds: 0.0,
            audio_file_path: None,
            message: "No audio checkpoints found".to_string(),
        });
    }

    // Scan for LEGACY session-0 checkpoint files only (sorted by filename).
    let checkpoint_files = legacy_session_chunk_files(&checkpoints_dir);

    if checkpoint_files.is_empty() {
        info!(
            "No checkpoint files found in: {}",
            checkpoints_dir.display()
        );
        return Ok(AudioRecoveryStatus {
            status: "none".to_string(),
            chunk_count: 0,
            estimated_duration_seconds: 0.0,
            audio_file_path: None,
            message: "No audio checkpoint files found".to_string(),
        });
    }

    let chunk_count = checkpoint_files.len() as u32;
    let estimated_duration = (chunk_count as f64) * 30.0; // 30 seconds per chunk

    info!(
        "Found {} checkpoint files, estimated duration: {:.2}s",
        chunk_count, estimated_duration
    );

    // Create FFmpeg concat file
    let concat_file_path = checkpoints_dir.join("concat_list.txt");
    let mut concat_content = String::new();

    for path in &checkpoint_files {
        let path = path
            .canonicalize()
            .map_err(|e| format!("Failed to canonicalize path: {}", e))?;
        concat_content.push_str(&format!("file '{}'\n", path.display()));
    }

    std::fs::write(&concat_file_path, concat_content)
        .map_err(|e| format!("Failed to write concat file: {}", e))?;

    // Run FFmpeg to merge chunks
    let output_path = folder_path.join("audio.mp4");
    let output_path_str = output_path
        .to_str()
        .ok_or("Invalid output path")?
        .to_string();

    let ffmpeg_path = find_ffmpeg_path()
        .ok_or_else(|| "FFmpeg not found. Please install FFmpeg to recover audio.".to_string())?;
    info!("Using FFmpeg at: {:?}", ffmpeg_path);

    let mut command = std::process::Command::new(ffmpeg_path);

    command.args([
        "-f",
        "concat",
        "-safe",
        "0",
        "-i",
        concat_file_path.to_str().unwrap(),
        "-c",
        "copy",
        "-y", // Overwrite if exists
        &output_path_str,
    ]);

    // Hide console window on Windows
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        command.creation_flags(CREATE_NO_WINDOW);
    }

    let ffmpeg_result = command.output();

    match ffmpeg_result {
        Ok(output) if output.status.success() => {
            // Clean up concat file
            let _ = std::fs::remove_file(concat_file_path);

            info!("Successfully recovered audio: {}", output_path_str);

            Ok(AudioRecoveryStatus {
                status: "success".to_string(),
                chunk_count,
                estimated_duration_seconds: estimated_duration,
                audio_file_path: Some(output_path_str),
                message: format!("Successfully recovered {} audio chunks", chunk_count),
            })
        }
        Ok(output) => {
            let error = String::from_utf8_lossy(&output.stderr);
            error!("FFmpeg recovery failed: {}", error);
            Ok(AudioRecoveryStatus {
                status: "failed".to_string(),
                chunk_count,
                estimated_duration_seconds: estimated_duration,
                audio_file_path: None,
                message: format!("FFmpeg failed: {}", error),
            })
        }
        Err(e) => {
            error!("Failed to run FFmpeg: {}", e);
            Ok(AudioRecoveryStatus {
                status: "failed".to_string(),
                chunk_count,
                estimated_duration_seconds: estimated_duration,
                audio_file_path: None,
                message: format!("Failed to run FFmpeg: {}", e),
            })
        }
    }
}

/// Clean up checkpoint files after successful recording or recovery
/// This command is called by the frontend after successful save to clean up checkpoint files
#[tauri::command]
pub async fn cleanup_checkpoints(meeting_folder: String) -> Result<(), String> {
    info!("Cleaning up checkpoints for folder: {}", meeting_folder);

    let folder_path = PathBuf::from(&meeting_folder);
    let checkpoints_dir = folder_path.join(".checkpoints");

    if checkpoints_dir.exists() {
        std::fs::remove_dir_all(&checkpoints_dir)
            .map_err(|e| format!("Failed to remove checkpoints directory: {}", e))?;
        info!("Successfully cleaned up checkpoints directory");
    } else {
        info!("No checkpoints directory to clean up");
    }

    Ok(())
}

/// Check if a meeting folder has audio checkpoint files
/// Returns true if .checkpoints/ directory exists and contains .mp4 files
#[tauri::command]
pub async fn has_audio_checkpoints(meeting_folder: String) -> Result<bool, String> {
    let folder_path = PathBuf::from(&meeting_folder);
    let checkpoints_dir = folder_path.join(".checkpoints");

    // Check if checkpoints directory exists
    if !checkpoints_dir.exists() {
        return Ok(false);
    }

    // Scan for .mp4 checkpoint files
    let has_mp4_files = std::fs::read_dir(&checkpoints_dir)
        .map_err(|e| format!("Failed to read checkpoints directory: {}", e))?
        .filter_map(|entry| entry.ok())
        .any(|entry| entry.path().extension().and_then(|s| s.to_str()) == Some("mp4"));

    Ok(has_mp4_files)
}

#[cfg(test)]
mod tests {
    use super::super::recording_state::DeviceType;
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_checkpoint_creation() {
        // Create temp meeting folder
        let temp_dir = tempdir().unwrap();
        let meeting_folder = temp_dir.path().join("Test_Meeting");
        std::fs::create_dir_all(&meeting_folder).unwrap();
        std::fs::create_dir_all(meeting_folder.join(".checkpoints")).unwrap();

        let mut saver = IncrementalAudioSaver::new(meeting_folder.clone(), 48000).unwrap();

        // Add 60 seconds worth of audio (should create 2 checkpoints)
        for i in 0..120 {
            // 120 chunks of 0.5s each
            let chunk = AudioChunk {
                data: vec![0.5f32; 24000], // 0.5s at 48kHz
                sample_rate: 48000,
                timestamp: i as f64 * 0.5, // timestamp in seconds
                chunk_id: i as u64,
                device_type: DeviceType::Microphone,
            };
            saver.add_chunk(chunk).unwrap();
        }

        // Verify 2 checkpoints created
        assert_eq!(saver.checkpoint_count, 2);

        // Finalize and verify merge
        let final_path = saver.finalize().await.unwrap();
        assert!(final_path.exists());

        // Verify checkpoints directory deleted
        assert!(!meeting_folder.join(".checkpoints").exists());
    }

    #[tokio::test]
    async fn test_empty_recording() {
        let temp_dir = tempdir().unwrap();
        let meeting_folder = temp_dir.path().join("Empty_Test");
        std::fs::create_dir_all(&meeting_folder).unwrap();
        std::fs::create_dir_all(meeting_folder.join(".checkpoints")).unwrap();

        let mut saver = IncrementalAudioSaver::new(meeting_folder.clone(), 48000).unwrap();

        // Try to finalize without adding any chunks
        let result = saver.finalize().await;
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("No audio checkpoints"));
    }

    /// Feed `seconds` of audio into the saver (0.5s chunks at 48 kHz).
    fn feed_audio(saver: &mut IncrementalAudioSaver, seconds: usize) {
        for i in 0..(seconds * 2) {
            let chunk = AudioChunk {
                data: vec![0.5f32; 24000], // 0.5s at 48kHz
                sample_rate: 48000,
                timestamp: i as f64 * 0.5,
                chunk_id: i as u64,
                device_type: DeviceType::Microphone,
            };
            saver.add_chunk(chunk).unwrap();
        }
    }

    /// FIX 1 (specs/0037): a RESUMED session's finalize must delete ONLY its own
    /// (prefix-scoped) chunks — chunks PRESERVED from a crashed prior session whose
    /// merge failed must survive, along with the `.checkpoints/` dir holding them.
    #[tokio::test]
    async fn resumed_finalize_preserves_foreign_checkpoint_chunks() {
        let temp_dir = tempdir().unwrap();
        let meeting_folder = temp_dir.path().join("Resumed_Meeting");
        let checkpoints = meeting_folder.join(".checkpoints");
        std::fs::create_dir_all(&checkpoints).unwrap();

        // Preserved chunks of a crashed session #0 whose recovery merge failed —
        // the ONLY copy of that audio.
        std::fs::write(checkpoints.join("audio_chunk_000.mp4"), b"crashed-seg0").unwrap();
        std::fs::write(checkpoints.join("audio_chunk_001.mp4"), b"crashed-seg0").unwrap();

        // The resumed session records segment 1.
        let mut saver =
            IncrementalAudioSaver::new_with_segment(meeting_folder.clone(), 48000, 1).unwrap();
        feed_audio(&mut saver, 35); // > 30s → at least one seg01_ checkpoint
        assert!(saver.get_checkpoint_count() >= 1);
        assert!(checkpoints.join("seg01_audio_chunk_000.mp4").exists());

        let final_path = saver.finalize().await.unwrap();
        assert_eq!(final_path, meeting_folder.join("audio_seg01.mp4"));
        assert!(final_path.exists());

        // Own chunks + concat list deleted; foreign chunks and the dir preserved.
        assert!(!checkpoints.join("seg01_audio_chunk_000.mp4").exists());
        assert!(!checkpoints.join("concat_list.txt").exists());
        assert!(checkpoints.join("audio_chunk_000.mp4").exists());
        assert!(checkpoints.join("audio_chunk_001.mp4").exists());
        assert!(
            checkpoints.exists(),
            ".checkpoints must survive (non-empty)"
        );
    }

    /// FIX 1 counterpart: a fresh (unprefixed) session with nothing foreign in
    /// `.checkpoints/` must still fully clean up, dir included — exactly the pre-0037
    /// behaviour (also asserted by `test_checkpoint_creation`).
    #[tokio::test]
    async fn fresh_finalize_fully_cleans_up_checkpoints_dir() {
        let temp_dir = tempdir().unwrap();
        let meeting_folder = temp_dir.path().join("Fresh_Meeting");
        let checkpoints = meeting_folder.join(".checkpoints");
        std::fs::create_dir_all(&checkpoints).unwrap();

        let mut saver = IncrementalAudioSaver::new(meeting_folder.clone(), 48000).unwrap();
        feed_audio(&mut saver, 35);

        let final_path = saver.finalize().await.unwrap();
        assert_eq!(final_path, meeting_folder.join("audio.mp4"));
        assert!(final_path.exists());
        assert!(
            !checkpoints.exists(),
            "empty .checkpoints must be removed for a fresh session"
        );
    }

    /// FIX 5 (specs/0037): legacy session-0 recovery must select ONLY the unprefixed
    /// `audio_chunk_*` chunks — a resumed session's `seg{NN}_` chunks are not its to merge.
    #[test]
    fn legacy_chunk_scan_excludes_segment_prefixed_chunks() {
        let temp_dir = tempdir().unwrap();
        let checkpoints = temp_dir.path().join(".checkpoints");
        std::fs::create_dir_all(&checkpoints).unwrap();
        std::fs::write(checkpoints.join("audio_chunk_001.mp4"), b"legacy1").unwrap();
        std::fs::write(checkpoints.join("audio_chunk_000.mp4"), b"legacy0").unwrap();
        std::fs::write(checkpoints.join("seg01_audio_chunk_000.mp4"), b"resumed").unwrap();
        std::fs::write(checkpoints.join("seg02_audio_chunk_000.mp4"), b"resumed").unwrap();
        std::fs::write(checkpoints.join("concat_list.txt"), b"not-a-chunk").unwrap();

        let chunks = legacy_session_chunk_files(&checkpoints);
        let names: Vec<String> = chunks
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        assert_eq!(names, vec!["audio_chunk_000.mp4", "audio_chunk_001.mp4"]);
    }

    #[test]
    fn legacy_chunk_scan_of_missing_dir_is_empty() {
        let temp_dir = tempdir().unwrap();
        assert!(legacy_session_chunk_files(&temp_dir.path().join("nope")).is_empty());
    }
}
