//! The Rust-side transcript copy of a live recording (specs/0083 W2a task 3).
//!
//! The `transcript-update` listener used to write each segment through the global
//! `RECORDING_MANAGER`. `stop_recording` `take()`s the manager out of that global, and it
//! also unlistened before awaiting the transcription drain. So every segment the drain
//! produced reached the webview but never `RecordingSaver.transcript_segments` or
//! `transcripts.json`.
//!
//! A [`TranscriptSink`] is a clone of the saver's own segment store (plus the folder and
//! the resumed sessions' prior segments), so the listener writes into it directly, with
//! no lookup through the global. [`drain_then_detach`] awaits the transcription task
//! FIRST, then removes the listener, then rewrites `transcripts.json` so the raw on-disk
//! capture holds the drained tail too.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Result;
use log::{error, info, warn};
use tauri::{AppHandle, Emitter, EventId, Listener, Runtime};

use super::recording_saver::TranscriptSegment;
use super::transcription::TranscriptUpdate;

/// The saver's transcript store, shareable with the `transcript-update` listener.
#[derive(Clone)]
pub struct TranscriptSink {
    segments: Arc<Mutex<Vec<TranscriptSegment>>>,
    /// Resumed sessions' segments (specs/0037), written before the current session's.
    prior: Arc<Vec<TranscriptSegment>>,
    folder: Option<PathBuf>,
}

impl TranscriptSink {
    pub fn new(
        segments: Arc<Mutex<Vec<TranscriptSegment>>>,
        prior: Vec<TranscriptSegment>,
        folder: Option<PathBuf>,
    ) -> Self {
        Self {
            segments,
            prior: Arc::new(prior),
            folder,
        }
    }

    /// Upsert a segment (by `sequence_id`) and save `transcripts.json` incrementally.
    pub fn add(&self, segment: TranscriptSegment) {
        upsert_segment(&self.segments, segment);
        if let Err(e) = self.write_json() {
            warn!("Failed to write incremental transcript update: {}", e);
        }
    }

    /// Rewrite `transcripts.json` from the current contents. No-op without a folder.
    pub fn write_json(&self) -> Result<()> {
        match &self.folder {
            Some(folder) => write_session_transcripts_json(folder, &self.prior, &self.segments),
            None => Ok(()),
        }
    }

    /// How many resumed sessions' segments precede this session's in `transcripts.json`.
    pub fn prior_len(&self) -> usize {
        self.prior.len()
    }

    /// The current session's segments (prior sessions excluded).
    pub fn segments(&self) -> Vec<TranscriptSegment> {
        self.segments.lock().map(|s| s.clone()).unwrap_or_default()
    }
}

/// Insert `segment`, or replace the one with the same `sequence_id`.
pub(crate) fn upsert_segment(segments: &Mutex<Vec<TranscriptSegment>>, segment: TranscriptSegment) {
    let Ok(mut segments) = segments.lock() else {
        error!(
            "Failed to lock transcript segments for adding segment {}",
            segment.id
        );
        return;
    };
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
        segments.push(segment.clone());
        info!(
            "Added new transcript segment {} (seq: {}) - total segments: {}",
            segment.id,
            segment.sequence_id,
            segments.len()
        );
    }
}

/// Write `transcripts.json` atomically (temp file + rename). Prior sessions first, then
/// the current session's segments, so a resumed session's write can never destroy the
/// prior session's only on-disk copy (specs/0037).
pub(crate) fn write_session_transcripts_json(
    folder: &Path,
    prior: &[TranscriptSegment],
    current: &Mutex<Vec<TranscriptSegment>>,
) -> Result<()> {
    // Clone to avoid holding the lock during I/O.
    let current_segments = match current.lock() {
        Ok(segments) => segments.clone(),
        Err(_) => {
            error!("Failed to lock transcript segments for writing");
            return Err(anyhow::anyhow!("Failed to lock transcript segments"));
        }
    };
    let mut all = prior.to_vec();
    all.extend(current_segments);

    info!(
        "Writing {} transcript segments to JSON ({} preserved from prior sessions)",
        all.len(),
        prior.len()
    );

    let transcript_path = folder.join("transcripts.json");
    let temp_path = folder.join(".transcripts.json.tmp");
    let json = serde_json::json!({
        "version": "1.0",
        "segments": all,
        "last_updated": chrono::Utc::now().to_rfc3339(),
        "total_segments": all.len()
    });
    let json_string = serde_json::to_string_pretty(&json).map_err(|e| {
        error!("Failed to serialize transcripts to JSON: {}", e);
        anyhow::anyhow!("JSON serialization failed: {}", e)
    })?;
    std::fs::write(&temp_path, &json_string).map_err(|e| {
        error!(
            "Failed to write transcript temp file to {}: {}",
            temp_path.display(),
            e
        );
        anyhow::anyhow!("Failed to write temp file: {}", e)
    })?;
    if !temp_path.exists() {
        error!(
            "Temp transcript file does not exist after write: {}",
            temp_path.display()
        );
        return Err(anyhow::anyhow!("Temp file verification failed"));
    }
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
        all.len()
    );
    Ok(())
}

/// The stored segment for one `transcript-update` payload.
pub(crate) fn segment_from_update(update: &TranscriptUpdate) -> TranscriptSegment {
    TranscriptSegment {
        id: format!("seg_{}", update.sequence_id),
        text: update.text.clone(),
        audio_start_time: update.audio_start_time,
        audio_end_time: update.audio_end_time,
        duration: update.duration,
        display_time: update.timestamp.clone(), // wall-clock timestamp for display
        confidence: update.confidence,
        sequence_id: update.sequence_id,
        // specs/0029 WS3.4: keep the capture-channel tag in the rehydration copy.
        channel: update.channel.clone(),
        word_timestamps: update.word_timestamps.clone(), // specs/0055 Stage 2
    }
}

/// Listen for `transcript-update` and write each segment into `sink`. `on_update` runs
/// after the write (the live diarizer feed). Returns the id to unlisten with.
pub(crate) fn listen_into_sink<R: Runtime>(
    app: &AppHandle<R>,
    sink: TranscriptSink,
    on_update: fn(&TranscriptUpdate),
) -> EventId {
    app.listen("transcript-update", move |event: tauri::Event| {
        if let Ok(update) = serde_json::from_str::<TranscriptUpdate>(event.payload()) {
            sink.add(segment_from_update(&update));
            on_update(&update);
        }
    })
}

/// How the transcription drain ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DrainOutcome {
    /// The task finished (cleanly or with an error) within the timeout.
    Completed,
    /// The timeout fired first; the tail may be incomplete.
    TimedOut,
    /// There was no transcription task (record-only / deferred mode).
    NoTask,
}

/// Await the transcription task (up to `timeout`), THEN remove the listener, THEN rewrite
/// `transcripts.json`. The order is the fix: every segment the drain emits is delivered to
/// the sink while the listener is still registered.
pub(crate) async fn drain_then_detach<R: Runtime>(
    app: &AppHandle<R>,
    task: Option<tokio::task::JoinHandle<()>>,
    listener: Option<EventId>,
    sink: Option<&TranscriptSink>,
    timeout: Duration,
) -> DrainOutcome {
    let outcome = match task {
        Some(handle) => await_drain(app, handle, timeout).await,
        None => {
            info!("ℹ️ No transcription task found to wait for");
            DrainOutcome::NoTask
        }
    };
    if let Some(id) = listener {
        app.unlisten(id);
        info!("✅ Transcript-update listener removed (after the drain)");
    }
    if let Some(sink) = sink {
        if let Err(e) = sink.write_json() {
            warn!("Failed to rewrite transcripts.json after the drain: {}", e);
        }
    }
    outcome
}

async fn await_drain<R: Runtime>(
    app: &AppHandle<R>,
    handle: tokio::task::JoinHandle<()>,
    timeout: Duration,
) -> DrainOutcome {
    info!("⏳ Waiting for ALL transcription chunks to be processed");
    let progress_app = app.clone();
    let progress_task = tokio::spawn(async move {
        let started = std::time::Instant::now();
        loop {
            tokio::time::sleep(Duration::from_millis(500)).await;
            let elapsed = started.elapsed().as_secs();
            let _ = progress_app.emit(
                "recording-shutdown-progress",
                serde_json::json!({
                    "stage": "processing_transcripts",
                    "message": format!("Processing transcripts... ({}s elapsed)", elapsed),
                    "progress": 40,
                    "detailed": true,
                    "elapsed_seconds": elapsed
                }),
            );
        }
    });
    let outcome = match tokio::time::timeout(timeout, handle).await {
        Ok(Ok(())) => {
            info!("✅ ALL transcription chunks processed successfully - no data lost");
            DrainOutcome::Completed
        }
        Ok(Err(e)) => {
            // Continue anyway - the worker may have processed most chunks.
            warn!("⚠️ Transcription task completed with error: {:?}", e);
            DrainOutcome::Completed
        }
        Err(_) => {
            warn!(
                "⏱️ Transcription timeout ({}s) reached, continuing shutdown to prevent indefinite hang",
                timeout.as_secs()
            );
            DrainOutcome::TimedOut
        }
    };
    progress_task.abort();
    outcome
}

#[cfg(test)]
#[path = "transcript_sink_tests.rs"]
mod tests;
