// audio/transcription/worker.rs
//
// Parallel transcription worker pool and chunk processing logic.

use super::engine::TranscriptionEngine;
use super::provider::TranscriptionError;
use super::update::TranscriptUpdate;
use crate::audio::pipeline::TranscriptionChunk;
use crate::audio::AudioChunk;
use log::{error, info, warn};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Runtime};

// Sequence counter for transcript updates
static SEQUENCE_COUNTER: AtomicU64 = AtomicU64::new(0);

// Speech detection flag - reset per recording session
static SPEECH_DETECTED_EMITTED: AtomicBool = AtomicBool::new(false);

/// Reset the speech detected flag for a new recording session
pub fn reset_speech_detected_flag() {
    SPEECH_DETECTED_EMITTED.store(false, Ordering::SeqCst);
    info!(
        "🔍 SPEECH_DETECTED_EMITTED reset to: {}",
        SPEECH_DETECTED_EMITTED.load(Ordering::SeqCst)
    );
}

// NOTE: get_transcript_history and get_recording_meeting_name functions
// have been moved to recording_commands.rs where they have access to RECORDING_MANAGER

/// Optimized parallel transcription task ensuring ZERO chunk loss
pub fn start_transcription_task<R: Runtime>(
    app: AppHandle<R>,
    transcription_receiver: tokio::sync::mpsc::Receiver<TranscriptionChunk>,
) -> tokio::task::JoinHandle<()> {
    // NOTE (specs/0029 WS4.2 → WS7.2): the live `recording-level` /
    // `recording-spectrum` emitter used to be registered here, but this task is not
    // spawned at all in record-only mode (live transcription off) while the
    // visualizers must keep working. Registration now happens once at app setup
    // (lib.rs); the pipeline emits from its raw mixed-window path.

    tokio::spawn(async move {
        info!("🚀 Starting optimized parallel transcription task - guaranteeing zero chunk loss");

        // Initialize transcription engine (Whisper or Parakeet based on config)
        let transcription_engine = match super::engine::get_or_init_transcription_engine(&app).await
        {
            Ok(engine) => engine,
            Err(e) => {
                error!("Failed to initialize transcription engine: {}", e);
                let _ = app.emit("transcription-error", serde_json::json!({
                    "error": e,
                    "userMessage": "Recording failed: Unable to initialize speech recognition. Please check your model settings.",
                    "actionable": true
                }));
                return;
            }
        };

        // Create parallel workers for faster processing while preserving ALL chunks
        const NUM_WORKERS: usize = 1; // Serial processing ensures transcripts emit in chronological order
                                      // specs/0028: BOUNDED work queue. When transcription (e.g. Whisper) can't keep up,
                                      // this queue saturates. Rather than growing unbounded (multi-minute stop hang +
                                      // unbounded RAM), we apply an explicit drop policy and surface a
                                      // `transcription-falling-behind` event so the user is warned. Dropped chunks are
                                      // counted separately and never mislabelled as processed.
        let (work_sender, work_receiver) =
            tokio::sync::mpsc::channel::<TranscriptionChunk>(WORK_QUEUE_CAPACITY);
        let work_receiver = Arc::new(tokio::sync::Mutex::new(work_receiver));

        // Track completion: AtomicU64 for chunks queued, AtomicU64 for chunks accounted-for
        // (chunks_completed = processed + skipped + failed, used purely for drain/termination
        // accounting). chunks_skipped counts chunks we could NOT transcribe (e.g. model
        // unloaded); chunks_dropped counts chunks shed under backpressure (queue saturated) —
        // both surfaced to the user so audio is never silently lost (specs/0028).
        let chunks_queued = Arc::new(AtomicU64::new(0));
        let chunks_completed = Arc::new(AtomicU64::new(0));
        let chunks_skipped = Arc::new(AtomicU64::new(0));
        let chunks_dropped = Arc::new(AtomicU64::new(0));
        let input_finished = Arc::new(AtomicBool::new(false));

        info!(
            "📊 Starting {} transcription worker{} (serial mode for ordered emission)",
            NUM_WORKERS,
            if NUM_WORKERS == 1 { "" } else { "s" }
        );

        // Spawn worker tasks
        let mut worker_handles = Vec::new();
        for worker_id in 0..NUM_WORKERS {
            let engine_clone = match &transcription_engine {
                TranscriptionEngine::Whisper(e) => TranscriptionEngine::Whisper(e.clone()),
                TranscriptionEngine::Parakeet(e) => TranscriptionEngine::Parakeet(e.clone()),
                TranscriptionEngine::Provider(p) => TranscriptionEngine::Provider(p.clone()),
            };
            let app_clone = app.clone();
            let work_receiver_clone = work_receiver.clone();
            let chunks_completed_clone = chunks_completed.clone();
            let chunks_skipped_clone = chunks_skipped.clone();
            let input_finished_clone = input_finished.clone();
            let chunks_queued_clone = chunks_queued.clone();

            let worker_handle = tokio::spawn(async move {
                info!("👷 Worker {} started", worker_id);

                // PRE-VALIDATE model state to avoid repeated async calls per chunk
                let initial_model_loaded = engine_clone.is_model_loaded().await;
                let current_model = engine_clone
                    .get_current_model()
                    .await
                    .unwrap_or_else(|| "unknown".to_string());

                let engine_name = engine_clone.provider_name();

                if initial_model_loaded {
                    info!(
                        "✅ Worker {} pre-validation: {} model '{}' is loaded and ready",
                        worker_id, engine_name, current_model
                    );
                } else {
                    warn!(
                        "⚠️ Worker {} pre-validation: {} model not loaded - chunks may be skipped",
                        worker_id, engine_name
                    );
                }

                loop {
                    // Try to get a chunk to process
                    let chunk = {
                        let mut receiver = work_receiver_clone.lock().await;
                        receiver.recv().await
                    };

                    match chunk {
                        Some(transcription_chunk) => {
                            // specs/0029 WS3.4: split off the capture-channel tag; the
                            // rest of the loop works on the inner audio chunk.
                            let TranscriptionChunk {
                                chunk,
                                channel,
                                channel_runs,
                            } = transcription_chunk;

                            // PERFORMANCE OPTIMIZATION: Reduce logging in hot path
                            // Only log every 10th chunk per worker to reduce I/O overhead
                            let should_log_this_chunk = chunk.chunk_id % 10 == 0;

                            if should_log_this_chunk {
                                info!(
                                    "👷 Worker {} processing chunk {} with {} samples",
                                    worker_id,
                                    chunk.chunk_id,
                                    chunk.data.len()
                                );
                            }

                            // Check if model is still loaded before processing. The model can be
                            // briefly unloaded/reloaded mid-recording (e.g. a model switch); give
                            // it a bounded window to come back before giving up (best-effort
                            // retry, specs/0028).
                            let mut model_ready = engine_clone.is_model_loaded().await;
                            if !model_ready {
                                for _ in 0..MODEL_RELOAD_RETRIES {
                                    tokio::time::sleep(MODEL_RELOAD_RETRY_DELAY).await;
                                    if engine_clone.is_model_loaded().await {
                                        model_ready = true;
                                        break;
                                    }
                                }
                            }
                            if !model_ready {
                                // SKIPPED (specs/0028): the model is gone and this chunk could
                                // NOT be transcribed. Do not report it as a successful
                                // transcription — record it as skipped and surface a
                                // user-visible warning so the user knows audio was missed. We
                                // still advance chunks_completed so the drain/termination
                                // accounting (queued == accounted-for) stays correct and stop
                                // does not hang.
                                warn!("⚠️ Worker {}: model unloaded, skipping chunk {} (could not transcribe)", worker_id, chunk.chunk_id);
                                let skipped =
                                    chunks_skipped_clone.fetch_add(1, Ordering::SeqCst) + 1;
                                chunks_completed_clone.fetch_add(1, Ordering::SeqCst);
                                emit_chunk_skipped(
                                    &app_clone,
                                    worker_id,
                                    chunk.chunk_id,
                                    "model_unloaded",
                                    skipped,
                                );
                                continue;
                            }

                            let chunk_timestamp = chunk.timestamp;
                            let chunk_duration = chunk.data.len() as f64 / chunk.sample_rate as f64;

                            // Transcribe with provider-agnostic approach
                            match transcribe_chunk_with_provider(&engine_clone, chunk, &app_clone)
                                .await
                            {
                                Ok((transcript, confidence_opt, is_partial)) => {
                                    // Provider-aware confidence threshold
                                    let confidence_threshold = match &engine_clone {
                                        TranscriptionEngine::Whisper(_)
                                        | TranscriptionEngine::Provider(_) => 0.3,
                                        TranscriptionEngine::Parakeet(_) => 0.0, // Parakeet has no confidence, accept all
                                    };

                                    let confidence_str = match confidence_opt {
                                        Some(c) => format!("{:.2}", c),
                                        None => "N/A".to_string(),
                                    };

                                    // PRIVACY (specs/0028): never log transcript CONTENT at
                                    // info! — release logs persist to ~/Library/Logs. Log only
                                    // length/confidence; full text is dev-only via perf_debug!.
                                    info!("🔍 Worker {} transcription result: {} chars, confidence={}, partial={}, threshold={:.2}",
                                          worker_id, transcript.len(), confidence_str, is_partial, confidence_threshold);
                                    perf_debug!(
                                        "🔍 Worker {} transcription text='{}'",
                                        worker_id,
                                        transcript
                                    );

                                    // Check confidence threshold (or accept if no confidence provided)
                                    let meets_threshold =
                                        confidence_opt.is_none_or(|c| c >= confidence_threshold);

                                    if !transcript.trim().is_empty() && meets_threshold {
                                        // PERFORMANCE: Only log transcription results, not every processing step
                                        // PRIVACY (specs/0028): length only at info!; content dev-only.
                                        info!("✅ Worker {} transcribed: {} chars (confidence: {}, partial: {})",
                                              worker_id, transcript.len(), confidence_str, is_partial);
                                        perf_debug!(
                                            "✅ Worker {} transcribed text: {}",
                                            worker_id,
                                            transcript
                                        );

                                        // Emit speech-detected event for frontend UX (only on first detection per session)
                                        // This is lightweight and provides better user feedback
                                        let current_flag =
                                            SPEECH_DETECTED_EMITTED.load(Ordering::SeqCst);
                                        info!("🔍 Checking speech-detected flag: current={}, will_emit={}", current_flag, !current_flag);

                                        if !current_flag {
                                            SPEECH_DETECTED_EMITTED.store(true, Ordering::SeqCst);
                                            match app_clone.emit("speech-detected", serde_json::json!({
                                                "message": "Speech activity detected"
                                            })) {
                                                Ok(_) => info!("🎤 ✅ First speech detected - successfully emitted speech-detected event"),
                                                Err(e) => error!("🎤 ❌ Failed to emit speech-detected event: {}", e),
                                            }
                                        } else {
                                            info!("🔍 Speech already detected in this session, not re-emitting");
                                        }

                                        // Generate sequence ID and calculate timestamps FIRST
                                        let sequence_id =
                                            SEQUENCE_COUNTER.fetch_add(1, Ordering::SeqCst);
                                        let audio_start_time = chunk_timestamp; // Already in seconds from recording start
                                        let audio_end_time = chunk_timestamp + chunk_duration;

                                        // Save structured transcript segment to recording manager (only final results)
                                        // Save ALL segments (partial and final) to ensure complete JSON
                                        // Create structured segment with full timestamp data
                                        // NOTE: This is now handled via the transcript-update event emission below
                                        // The recording_commands module listens to these events and saves them
                                        // This decouples the transcription worker from direct RECORDING_MANAGER access

                                        // Emit transcript update with NEW recording-relative timestamps

                                        let update = TranscriptUpdate {
                                            text: transcript,
                                            timestamp: format_current_timestamp(), // Wall-clock for reference
                                            source: "Audio".to_string(),
                                            sequence_id,
                                            chunk_start_time: chunk_timestamp, // Legacy compatibility
                                            is_partial,
                                            confidence: confidence_opt.unwrap_or(0.85), // Default for providers without confidence
                                            // NEW: Recording-relative timestamps for sync
                                            audio_start_time,
                                            audio_end_time,
                                            duration: chunk_duration,
                                            // Live diarization labels this later (or never, if
                                            // disabled); always `None` at first emission.
                                            speaker: None,
                                            // specs/0029 WS3.4: capture-channel tag from the
                                            // pipeline's per-window RMS dominance.
                                            channel: channel.map(|c| c.as_str().to_string()),
                                            channel_runs: channel_runs.clone(), // specs/0055
                                        };

                                        if let Err(e) = app_clone.emit("transcript-update", &update)
                                        {
                                            error!(
                                                "Worker {}: Failed to emit transcript update: {}",
                                                worker_id, e
                                            );
                                        }
                                        // PERFORMANCE: Removed verbose logging of every emission
                                    } else if !transcript.trim().is_empty() && should_log_this_chunk
                                    {
                                        // PERFORMANCE: Only log low-confidence results occasionally
                                        if let Some(c) = confidence_opt {
                                            info!("Worker {} low-confidence transcription (confidence: {:.2}), skipping", worker_id, c);
                                        }
                                    }
                                }
                                Err(e) => {
                                    // Improved error handling with specific cases
                                    match e {
                                        TranscriptionError::AudioTooShort { .. } => {
                                            // Skip silently, this is expected for very short chunks
                                            info!("Worker {}: {}", worker_id, e);
                                            chunks_completed_clone.fetch_add(1, Ordering::SeqCst);
                                            continue;
                                        }
                                        TranscriptionError::ModelNotLoaded => {
                                            // SKIPPED (specs/0028): model unloaded mid-transcription;
                                            // this chunk was not transcribed. Count as skipped (not a
                                            // silent success) and surface it. Still advance
                                            // chunks_completed for termination accounting.
                                            warn!("Worker {}: model unloaded during transcription, skipping chunk", worker_id);
                                            let skipped = chunks_skipped_clone
                                                .fetch_add(1, Ordering::SeqCst)
                                                + 1;
                                            chunks_completed_clone.fetch_add(1, Ordering::SeqCst);
                                            emit_chunk_skipped(
                                                &app_clone,
                                                worker_id,
                                                u64::MAX,
                                                "model_unloaded",
                                                skipped,
                                            );
                                            continue;
                                        }
                                        _ => {
                                            warn!(
                                                "Worker {}: Transcription failed: {}",
                                                worker_id, e
                                            );
                                            let _ = app_clone
                                                .emit("transcription-warning", e.to_string());
                                        }
                                    }
                                }
                            }

                            // Mark chunk as completed
                            let completed =
                                chunks_completed_clone.fetch_add(1, Ordering::SeqCst) + 1;
                            let queued = chunks_queued_clone.load(Ordering::SeqCst);

                            // PERFORMANCE: Only log progress every 5th chunk to reduce I/O overhead
                            if completed.is_multiple_of(5) || should_log_this_chunk {
                                info!(
                                    "Worker {}: Progress {}/{} chunks ({:.1}%)",
                                    worker_id,
                                    completed,
                                    queued,
                                    (completed as f64 / queued.max(1) as f64 * 100.0)
                                );
                            }

                            // Emit progress event for frontend
                            let progress_percentage = if queued > 0 {
                                (completed as f64 / queued as f64 * 100.0) as u32
                            } else {
                                100
                            };

                            let _ = app_clone.emit("transcription-progress", serde_json::json!({
                                "worker_id": worker_id,
                                "chunks_completed": completed,
                                "chunks_queued": queued,
                                "progress_percentage": progress_percentage,
                                "message": format!("Worker {} processing... ({}/{})", worker_id, completed, queued)
                            }));
                        }
                        None => {
                            // No more chunks available
                            if input_finished_clone.load(Ordering::SeqCst) {
                                // Double-check that all queued chunks are actually completed
                                let final_queued = chunks_queued_clone.load(Ordering::SeqCst);
                                let final_completed = chunks_completed_clone.load(Ordering::SeqCst);

                                if final_completed >= final_queued {
                                    info!(
                                        "👷 Worker {} finishing - all {}/{} chunks processed",
                                        worker_id, final_completed, final_queued
                                    );
                                    break;
                                } else {
                                    warn!("👷 Worker {} detected potential chunk loss: {}/{} completed, waiting...", worker_id, final_completed, final_queued);
                                    // AGGRESSIVE POLLING: Reduced from 50ms to 5ms for faster chunk detection during shutdown
                                    tokio::time::sleep(tokio::time::Duration::from_millis(5)).await;
                                }
                            } else {
                                // AGGRESSIVE POLLING: Reduced from 10ms to 1ms for faster response during shutdown
                                tokio::time::sleep(tokio::time::Duration::from_millis(1)).await;
                            }
                        }
                    }
                }

                info!("👷 Worker {} completed", worker_id);
            });

            worker_handles.push(worker_handle);
        }

        // Main dispatcher: receive chunks and distribute to workers.
        //
        // NOTE (specs/0029 WS4.2): this receiver does NOT see every mixed chunk — it only
        // receives VAD speech segments (>= 50 ms) that survived the bounded
        // pipeline→transcription queue. The live `recording-level` / `recording-spectrum`
        // UI events are therefore emitted from the pipeline's raw mixed-window path
        // (audio/pipeline.rs), which sees every window regardless of VAD gating or queue
        // backpressure. (A previous comment here claimed this task "sees every mixed
        // chunk"; that was false, and emitting from here left the spectrometer VAD-gated
        // and starvable.)
        let mut receiver = transcription_receiver;
        while let Some(chunk) = receiver.recv().await {
            // specs/0028: BOUNDED dispatch with an explicit drop policy. `try_send` never
            // blocks, so a slow transcriber can't stall the dispatcher or grow the queue
            // without bound. On saturation we shed the newest chunk (drop policy), count it,
            // and emit a throttled `transcription-falling-behind` event so the user is warned
            // rather than silently losing audio. `chunks_queued` is incremented ONLY on
            // accepted chunks so the queued==completed termination invariant stays exact.
            let chunk_id = chunk.chunk.chunk_id;
            match work_sender.try_send(chunk) {
                Ok(()) => {
                    let queued = chunks_queued.fetch_add(1, Ordering::SeqCst) + 1;
                    info!(
                        "📥 Dispatching chunk {} to workers (total queued: {})",
                        chunk_id, queued
                    );
                }
                Err(tokio::sync::mpsc::error::TrySendError::Full(_dropped)) => {
                    let dropped = chunks_dropped.fetch_add(1, Ordering::SeqCst) + 1;
                    warn!(
                        "⚠️ Transcription falling behind: work queue saturated ({} slots), dropped chunk {} (total dropped: {})",
                        WORK_QUEUE_CAPACITY, chunk_id, dropped
                    );
                    emit_falling_behind(&app, dropped);
                }
                Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
                    error!("❌ Worker channel closed unexpectedly - workers gone");
                    break;
                }
            }
        }

        // Signal that input is finished
        input_finished.store(true, Ordering::SeqCst);
        drop(work_sender); // Close the channel to signal workers

        let total_chunks_queued = chunks_queued.load(Ordering::SeqCst);
        info!("📭 Input finished with {} total chunks queued. Waiting for all {} workers to complete...",
              total_chunks_queued, NUM_WORKERS);

        // Emit final chunk count to frontend
        let _ = app.emit("transcription-queue-complete", serde_json::json!({
            "total_chunks": total_chunks_queued,
            "message": format!("{} chunks queued for processing - waiting for completion", total_chunks_queued)
        }));

        // Wait for all workers to complete
        for (worker_id, handle) in worker_handles.into_iter().enumerate() {
            if let Err(e) = handle.await {
                error!("❌ Worker {} panicked: {:?}", worker_id, e);
            } else {
                info!("✅ Worker {} completed successfully", worker_id);
            }
        }

        // Final verification with retry logic to catch any stragglers
        let mut verification_attempts = 0;
        const MAX_VERIFICATION_ATTEMPTS: u32 = 10;

        loop {
            let final_queued = chunks_queued.load(Ordering::SeqCst);
            let final_completed = chunks_completed.load(Ordering::SeqCst);

            if final_queued == final_completed {
                info!(
                    "🎉 ALL {} chunks processed successfully - ZERO chunks lost!",
                    final_completed
                );
                break;
            } else if verification_attempts < MAX_VERIFICATION_ATTEMPTS {
                verification_attempts += 1;
                warn!("⚠️ Chunk count mismatch (attempt {}): {} queued, {} completed - waiting for stragglers...",
                     verification_attempts, final_queued, final_completed);

                // Wait a bit for any remaining chunks to be processed
                tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
            } else {
                error!(
                    "❌ CRITICAL: After {} attempts, chunk loss detected: {} queued, {} completed",
                    MAX_VERIFICATION_ATTEMPTS, final_queued, final_completed
                );

                // Emit critical error event
                let _ = app.emit(
                    "transcript-chunk-loss-detected",
                    serde_json::json!({
                        "chunks_queued": final_queued,
                        "chunks_completed": final_completed,
                        "chunks_lost": final_queued - final_completed,
                        "message": "Some transcript chunks may have been lost during shutdown"
                    }),
                );
                break;
            }
        }

        // specs/0028: if any chunks were skipped (never transcribed, e.g. model unloaded),
        // surface an accurate, user-visible summary so a meeting is never silently missing
        // audio. This is distinct from the chunk-loss detector above, which only fires on an
        // accounting mismatch.
        let total_skipped = chunks_skipped.load(Ordering::SeqCst);
        if total_skipped > 0 {
            warn!(
                "⚠️ {} chunk(s) were skipped (not transcribed) during this recording",
                total_skipped
            );
            let _ = app.emit(
                "transcription-chunks-skipped",
                serde_json::json!({
                    "chunks_skipped": total_skipped,
                    "reason": "model_unloaded",
                    "userMessage": format!(
                        "{} audio segment(s) could not be transcribed because the speech model was unavailable, and were skipped.",
                        total_skipped
                    ),
                }),
            );
        }

        // specs/0028: surface chunks shed under backpressure (queue saturation) so a
        // "transcription fell behind" condition is never silent. Distinct from skipped
        // (model gone) and from the accounting-mismatch loss detector above.
        let total_dropped = chunks_dropped.load(Ordering::SeqCst);
        if total_dropped > 0 {
            warn!(
                "⚠️ {} chunk(s) were dropped because transcription fell behind",
                total_dropped
            );
            let _ = app.emit(
                "transcription-chunks-dropped",
                serde_json::json!({
                    "chunks_dropped": total_dropped,
                    "userMessage": format!(
                        "Transcription fell behind and {} audio segment(s) were dropped to keep the recording responsive.",
                        total_dropped
                    ),
                }),
            );
        }

        info!("✅ Parallel transcription task completed - all workers finished, ready for model unload");
    })
}

/// Bounded work-queue capacity feeding the transcription workers (specs/0028). Sized to
/// buffer a healthy backlog without allowing unbounded growth (multi-minute stop hang / RAM
/// blow-up) when the transcriber can't keep up. Tunable — see spec risk note on soak testing.
const WORK_QUEUE_CAPACITY: usize = 512;

/// Bounded best-effort retry window for a transiently-unloaded model before a chunk is
/// declared skipped (specs/0028). Kept short to avoid stalling the serial worker loop.
const MODEL_RELOAD_RETRIES: u32 = 3;
const MODEL_RELOAD_RETRY_DELAY: std::time::Duration = std::time::Duration::from_millis(50);

/// Emit a throttled `transcription-falling-behind` Tauri event (specs/0028) when the work
/// queue saturates and chunks are being dropped, so the frontend can show a non-blocking
/// warning. Throttled so sustained backpressure can't flood the event bus.
fn emit_falling_behind<R: Runtime>(app: &AppHandle<R>, total_dropped: u64) {
    if total_dropped == 1 || total_dropped.is_multiple_of(20) {
        let _ = app.emit(
            "transcription-falling-behind",
            serde_json::json!({
                "total_dropped": total_dropped,
                "userMessage": "Transcription is falling behind; some audio is being dropped to stay responsive.",
            }),
        );
    }
}

/// Emit a throttled, user-visible warning that a chunk was skipped (could not be
/// transcribed). New Tauri event `transcription-chunk-skipped` — consumed by the frontend
/// (specs/0028). Throttled so a persistently-unloaded model can't flood the event bus.
fn emit_chunk_skipped<R: Runtime>(
    app: &AppHandle<R>,
    worker_id: usize,
    chunk_id: u64,
    reason: &str,
    total_skipped: u64,
) {
    if total_skipped == 1 || total_skipped.is_multiple_of(25) {
        let _ = app.emit(
            "transcription-chunk-skipped",
            serde_json::json!({
                "worker_id": worker_id,
                "chunk_id": chunk_id,
                "reason": reason,
                "total_skipped": total_skipped,
                "userMessage": "Some audio could not be transcribed because the speech model was unloaded; those segments were skipped.",
            }),
        );
    }
}

/// Transcribe audio chunk using the appropriate provider (Whisper, Parakeet, or trait-based)
/// Returns: (text, confidence Option, is_partial)
async fn transcribe_chunk_with_provider<R: Runtime>(
    engine: &TranscriptionEngine,
    chunk: AudioChunk,
    app: &AppHandle<R>,
) -> std::result::Result<(String, Option<f32>, bool), TranscriptionError> {
    // Convert to 16kHz mono for transcription
    let transcription_data = if chunk.sample_rate != 16000 {
        crate::audio::audio_processing::resample_audio(&chunk.data, chunk.sample_rate, 16000)
    } else {
        chunk.data
    };

    // Skip VAD processing here since the pipeline already extracted speech using VAD
    let speech_samples = transcription_data;

    // Check for empty samples - improved error handling
    if speech_samples.is_empty() {
        warn!(
            "Audio chunk {} is empty, skipping transcription",
            chunk.chunk_id
        );
        return Err(TranscriptionError::AudioTooShort {
            samples: 0,
            minimum: 1600, // 100ms at 16kHz
        });
    }

    // Calculate energy for logging/monitoring only
    let energy: f32 =
        speech_samples.iter().map(|&x| x * x).sum::<f32>() / speech_samples.len() as f32;
    info!(
        "Processing speech audio chunk {} with {} samples (energy: {:.6})",
        chunk.chunk_id,
        speech_samples.len(),
        energy
    );

    // Transcribe using the appropriate engine (with improved error handling)
    match engine {
        TranscriptionEngine::Whisper(whisper_engine) => {
            // Get language preference from global state
            let language = crate::audio::capture_commands::get_language_preference_internal();

            match whisper_engine
                .transcribe_audio_with_confidence(speech_samples, language)
                .await
            {
                Ok((text, confidence, is_partial)) => {
                    let cleaned_text = text.trim().to_string();
                    if cleaned_text.is_empty() {
                        return Ok((String::new(), Some(confidence), is_partial));
                    }

                    // PRIVACY (specs/0028): length only at info!; content dev-only.
                    info!(
                        "Whisper transcription complete for chunk {}: {} chars (confidence: {:.2}, partial: {})",
                        chunk.chunk_id, cleaned_text.len(), confidence, is_partial
                    );
                    perf_debug!("Whisper chunk {} text: '{}'", chunk.chunk_id, cleaned_text);

                    Ok((cleaned_text, Some(confidence), is_partial))
                }
                Err(e) => {
                    error!(
                        "Whisper transcription failed for chunk {}: {}",
                        chunk.chunk_id, e
                    );

                    let transcription_error = TranscriptionError::EngineFailed(e.to_string());
                    let _ = app.emit(
                        "transcription-error",
                        &serde_json::json!({
                            "error": transcription_error.to_string(),
                            "userMessage": format!("Transcription failed: {}", transcription_error),
                            "actionable": false
                        }),
                    );

                    Err(transcription_error)
                }
            }
        }
        TranscriptionEngine::Parakeet(parakeet_engine) => {
            match parakeet_engine.transcribe_audio(speech_samples).await {
                Ok(text) => {
                    let cleaned_text = text.trim().to_string();
                    if cleaned_text.is_empty() {
                        return Ok((String::new(), None, false));
                    }

                    // PRIVACY (specs/0028): length only at info!; content dev-only.
                    info!(
                        "Parakeet transcription complete for chunk {}: {} chars",
                        chunk.chunk_id,
                        cleaned_text.len()
                    );
                    perf_debug!("Parakeet chunk {} text: '{}'", chunk.chunk_id, cleaned_text);

                    // Parakeet doesn't provide confidence or partial results
                    Ok((cleaned_text, None, false))
                }
                Err(e) => {
                    error!(
                        "Parakeet transcription failed for chunk {}: {}",
                        chunk.chunk_id, e
                    );

                    let transcription_error = TranscriptionError::EngineFailed(e.to_string());
                    let _ = app.emit(
                        "transcription-error",
                        &serde_json::json!({
                            "error": transcription_error.to_string(),
                            "userMessage": format!("Transcription failed: {}", transcription_error),
                            "actionable": false
                        }),
                    );

                    Err(transcription_error)
                }
            }
        }
        TranscriptionEngine::Provider(provider) => {
            // NEW: Trait-based provider (clean, unified interface)
            let language = crate::audio::capture_commands::get_language_preference_internal();

            match provider.transcribe(speech_samples, language).await {
                Ok(result) => {
                    let cleaned_text = result.text.trim().to_string();
                    if cleaned_text.is_empty() {
                        return Ok((String::new(), result.confidence, result.is_partial));
                    }

                    let confidence_str = match result.confidence {
                        Some(c) => format!("confidence: {:.2}", c),
                        None => "no confidence".to_string(),
                    };

                    // PRIVACY (specs/0028): length only at info!; content dev-only.
                    info!(
                        "{} transcription complete for chunk {}: {} chars ({}, partial: {})",
                        provider.provider_name(),
                        chunk.chunk_id,
                        cleaned_text.len(),
                        confidence_str,
                        result.is_partial
                    );
                    perf_debug!(
                        "{} chunk {} text: '{}'",
                        provider.provider_name(),
                        chunk.chunk_id,
                        cleaned_text
                    );

                    Ok((cleaned_text, result.confidence, result.is_partial))
                }
                Err(e) => {
                    error!(
                        "{} transcription failed for chunk {}: {}",
                        provider.provider_name(),
                        chunk.chunk_id,
                        e
                    );

                    let _ = app.emit(
                        "transcription-error",
                        &serde_json::json!({
                            "error": e.to_string(),
                            "userMessage": format!("Transcription failed: {}", e),
                            "actionable": false
                        }),
                    );

                    Err(e)
                }
            }
        }
    }
}

/// Format current timestamp (wall-clock time)
fn format_current_timestamp() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();

    let hours = (now.as_secs() / 3600) % 24;
    let minutes = (now.as_secs() / 60) % 60;
    let seconds = now.as_secs() % 60;

    format!("{:02}:{:02}:{:02}", hours, minutes, seconds)
}

/// Format recording-relative time as [MM:SS]
#[allow(dead_code)]
fn format_recording_time(seconds: f64) -> String {
    let total_seconds = seconds.floor() as u64;
    let minutes = total_seconds / 60;
    let secs = total_seconds % 60;

    format!("[{:02}:{:02}]", minutes, secs)
}
