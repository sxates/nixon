// src/audio/mod.rs
pub mod audio_processing;
pub mod decoder;
pub mod encode;
pub mod ffmpeg;
pub mod vad;
pub mod vad_split;

// Modularized device management
pub mod capture;
pub mod devices;
pub mod permissions;
pub mod probe_tone; // specs/0061 W3: sine-tone generator + playback for the honest Audio Capture probe

// NEW: Device detection and diagnostics for adaptive buffering
pub mod device_detection;
pub mod diagnostics;
pub mod ffmpeg_mixer; // NEW: FFmpeg-style adaptive audio mixer

// New simplified audio system
pub mod async_logger;
pub mod batch_processor;
pub mod buffer_pool;
pub mod capture_commands; // specs/0042 WS1: top-level recording/device/language commands (from lib.rs)
pub mod channel_attribution; // specs/0029 WS3.4 / 0055: per-window capture-channel classification + per-segment views
pub mod channel_writer; // NEW (specs/0010): per-channel 16kHz mono WAVs for diarization
pub mod deferred_backlog; // low-power-mode spec §5: backlog query for meetings awaiting deferred processing
pub mod device_monitor; // NEW: Device disconnect/reconnect monitoring
pub mod device_resolution; // low-power-mode ratchet offset: start-time mic/system device resolution
pub mod folder_lease; // specs/0073 W1: per-meeting exclusive right to touch a meeting folder
pub mod hardware_detector;
pub mod incremental_saver; // NEW: Incremental audio saving with checkpoints
pub mod level_monitor;
pub mod live_meter; // specs/0057 §3.2: per-channel + mixed live meter feed (split from pipeline.rs)
pub mod live_toggle; // low-power-mode spec §§3-4: session live/defer toggle state
pub mod meeting_folder; // specs/0073 W1: ownership check + folder copy helper
pub mod mute_gate; // specs/0049: owner-mic gate for Zoom-mute
pub mod pipeline;
pub mod playback_monitor;
pub mod post_processor;
pub mod processing_mode; // NEW (low-power-mode spec §§2-3): effective live-STT decision
pub mod processing_reconcile; // spec 0051 WS2: startup 'live' → 'defer' sweep
pub mod recording_commands;
pub mod recording_manager;
pub mod recording_preferences;
pub mod recording_recovery; // specs/0037: crash/quit recovery scan for resume
pub mod recording_saver;
pub mod recording_duration;
pub mod recording_state;
pub mod recordings_move; // specs/0073 W2: move every meeting when the recordings folder changes
pub mod retranscription_channels; // 1.10 feedback: channel tags for batch retranscription (deferred "You" attribution)
pub mod retranscription_engines; // engine get-or-init for batch retranscription (size-ratchet split)
pub mod simple_level_monitor;
pub mod stream;
pub mod stt_lock; // spec 0045 WS1b: process-wide inference mutex serializing live + batch STT decode
pub mod stt_stage; // low-power-mode spec §3: pipeline VAD/STT gating stage (attach/detach)
pub mod system_audio_commands;
pub mod system_detector; // NEW: Playback device detection for BT warnings
pub mod volume_check; // specs/0073: recordings stay on an internal, local drive

// Transcription module (provider abstraction, engine management, worker pool)
pub mod transcription;

// Shared utilities for import and retranscription
pub(crate) mod common;

// Shared constants
pub mod constants;

// Retranscription module (re-process stored audio with different settings)
pub mod retranscription;

// Audio lifecycle (specs/0072): processed state, retention policy, sweep + compression.
pub mod lifecycle;

// Import module (import external audio files as new meetings)
pub mod import;
pub(crate) mod import_engines; // engine lifecycle for import (specs/0042 ratchet split)

pub use devices::{
    default_input_device, default_output_device, get_device_and_config, list_audio_devices,
    parse_audio_device, trigger_audio_permission, AudioDevice, AudioTranscriptionEngine,
    DeviceControl, DeviceType, LAST_AUDIO_CAPTURE,
};

// Export system audio capture functionality
pub use capture::{
    check_system_audio_permissions, list_system_audio_devices, start_system_audio_capture,
    SystemAudioCapture, SystemAudioStream,
};

// Export system audio detection functionality
pub use system_detector::{
    new_system_audio_callback, SystemAudioCallback, SystemAudioDetector, SystemAudioEvent,
};

// Export system audio commands
pub use system_audio_commands::{
    check_system_audio_permissions_command, get_system_audio_monitoring_status,
    init_system_audio_state, list_system_audio_devices_command, start_system_audio_capture_command,
    start_system_audio_monitoring, stop_system_audio_monitoring,
};

// Export new simplified components
pub use pipeline::AudioPipelineManager;
pub use recording_state::{
    AudioChunk, AudioError, DeviceType as RecordingDeviceType, ProcessedAudioChunk, RecordingState,
};

// specs/0010 P1-B1: per-channel diarization WAV resolvers (the next slice — the
// diarization pipeline — uses these to locate a finished meeting's channels).
pub use buffer_pool::{AudioBufferPool, PooledBuffer};
pub use channel_writer::{mic_channel_wav, system_channel_wav};
pub use device_monitor::{AudioDeviceMonitor, DeviceEvent, DeviceMonitorType};
pub use encode::{encode_single_audio, AudioInput};
pub use hardware_detector::{AdaptiveWhisperConfig, GpuType, HardwareProfile, PerformanceTier};
pub use level_monitor::{AudioLevelData, AudioLevelMonitor, AudioLevelUpdate};
pub use post_processor::{PostProcessRequest, PostProcessResponse, PostProcessor};
pub use recording_commands::{
    get_transcription_status, is_recording, start_recording, start_recording_with_devices,
    stop_recording, RecordingArgs, TranscriptUpdate, TranscriptionStatus,
};
pub use recording_manager::RecordingManager;
pub use recording_preferences::{init_recordings_root, recordings_root, RecordingPreferences};
pub use recording_saver::RecordingSaver;
pub use stream::AudioStreamManager;

// Export device detection and diagnostics
pub use device_detection::{calculate_buffer_timeout, InputDeviceKind};
pub use diagnostics::{
    log_buffer_health, log_detection_summary, log_device_capabilities, log_mixer_status,
    log_performance_summary,
};

// Export FFmpeg mixer
pub use ffmpeg_mixer::{BufferStats, FFmpegAudioMixer, RNNOISE_APPLY_ENABLED};

pub use vad::extract_speech_16k;

// Export decoder for retranscription
pub use decoder::{decode_audio_file, DecodedAudio};

// Export audio constants
pub use constants::AUDIO_EXTENSIONS;
