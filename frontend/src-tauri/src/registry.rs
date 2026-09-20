// src/registry.rs
//
// Central Tauri command registry (specs/0042 WS1). Every #[tauri::command] the
// frontend can invoke is listed here, grouped by subsystem — lib.rs calls
// `registry::invoke_handler()` once and no longer changes when commands are
// added. The frontend-visible command name is the last path segment, so moving
// a command between modules never changes its `invoke("name")` string.
//
// To add a command: define it in its feature module, then append it to the
// matching section below.

use crate::{
    action_items, aggregation, anthropic, audio, calendar, database, diarization, fs_guard, groq,
    llm_activity, meetings, notifications, ollama, onboarding, onboarding_disk, openai, openrouter,
    parakeet_engine, people, power, search, settings, summary, transcripts, updater, utils,
    whisper_engine, zoom,
};
// specs/0059: the whole `dev_fixtures` module is `#![cfg(debug_assertions)]`-gated, so
// this import must be too — a release build has no `crate::dev_fixtures` to resolve.
#[cfg(debug_assertions)]
use crate::dev_fixtures;

// Concrete over `tauri::Wry` (not generic over `R: Runtime`): several registered
// commands take a plain `AppHandle` / `State<...<tauri::Wry>>`, so a generic
// handler doesn't satisfy `CommandArg<'_, R>`. `tauri::Builder::default()` is
// `Builder<Wry>`, so this is the only runtime the app ever constructs.
pub fn invoke_handler() -> impl Fn(tauri::ipc::Invoke<tauri::Wry>) -> bool + Send + Sync + 'static {
    tauri::generate_handler![
        // Recording lifecycle + device/language commands (audio/capture_commands.rs)
        audio::capture_commands::start_recording,
        audio::capture_commands::stop_recording,
        audio::capture_commands::is_recording,
        audio::capture_commands::get_transcription_status,
        // Webview-confined file access (specs/0028)
        fs_guard::read_audio_file,
        fs_guard::save_transcript,
        // Whisper engine commands
        whisper_engine::commands::whisper_init,
        whisper_engine::commands::whisper_get_available_models,
        whisper_engine::commands::whisper_load_model,
        whisper_engine::commands::whisper_get_current_model,
        whisper_engine::commands::whisper_is_model_loaded,
        whisper_engine::commands::whisper_has_available_models,
        whisper_engine::commands::whisper_validate_model_ready,
        whisper_engine::commands::whisper_transcribe_audio,
        whisper_engine::commands::whisper_get_models_directory,
        whisper_engine::commands::whisper_download_model,
        whisper_engine::commands::whisper_cancel_download,
        whisper_engine::commands::whisper_delete_corrupted_model,
        // Parakeet engine commands
        parakeet_engine::commands::parakeet_init,
        parakeet_engine::commands::parakeet_get_available_models,
        parakeet_engine::commands::parakeet_load_model,
        parakeet_engine::commands::parakeet_get_current_model,
        parakeet_engine::commands::parakeet_is_model_loaded,
        parakeet_engine::commands::parakeet_has_available_models,
        parakeet_engine::commands::parakeet_validate_model_ready,
        parakeet_engine::commands::parakeet_transcribe_audio,
        parakeet_engine::commands::parakeet_get_models_directory,
        parakeet_engine::commands::parakeet_download_model,
        parakeet_engine::commands::parakeet_retry_download,
        parakeet_engine::commands::parakeet_cancel_download,
        parakeet_engine::commands::parakeet_delete_corrupted_model,
        parakeet_engine::commands::open_parakeet_models_folder,
        // Parallel processing commands
        audio::capture_commands::get_audio_devices,
        audio::capture_commands::trigger_microphone_permission,
        audio::capture_commands::start_recording_with_devices_and_meeting,
        audio::capture_commands::start_audio_level_monitoring,
        audio::capture_commands::stop_audio_level_monitoring,
        audio::capture_commands::is_audio_level_monitoring,
        // Recording pause/resume commands
        audio::recording_commands::pause_recording,
        audio::recording_commands::resume_recording,
        audio::recording_commands::is_recording_paused,
        audio::recording_commands::get_recording_state,
        audio::recording_commands::get_meeting_folder_path,
        // specs/0037: crash/quit recovery — resume an interrupted recording
        audio::recording_recovery::api_list_interrupted_recordings,
        audio::recording_recovery::api_discard_interrupted_recording,
        // Reload sync commands (retrieve transcript history and meeting name)
        audio::recording_commands::get_transcript_history,
        audio::recording_commands::get_recording_meeting_name,
        // Device monitoring commands (AirPods/Bluetooth disconnect/reconnect)
        audio::recording_commands::poll_audio_device_events,
        audio::recording_commands::get_reconnection_status,
        audio::recording_commands::attempt_device_reconnect,
        // Playback device detection (Bluetooth warning)
        audio::recording_commands::get_active_audio_output,
        // Audio recovery commands (for transcript recovery feature)
        audio::incremental_saver::recover_audio_from_checkpoints,
        audio::incremental_saver::cleanup_checkpoints,
        audio::incremental_saver::has_audio_checkpoints,
        // Power state (low-power mode)
        power::api_get_power_state,
        // low-power-mode spec §3: mid-meeting go-live / go-defer toggle
        audio::live_toggle::api_apply_live_transcription_now,
        // low-power-mode spec §5: late-mount/remount hydration of session mode
        audio::live_toggle::api_get_session_processing_state,
        // LLM provider model listings
        ollama::get_ollama_models,
        ollama::pull_ollama_model,
        ollama::delete_ollama_model,
        ollama::get_ollama_model_context,
        openai::openai::get_openai_models,
        anthropic::anthropic::get_anthropic_models,
        groq::groq::get_groq_models,
        // Meetings / settings / transcripts API
        meetings::commands::api_get_meetings,
        meetings::calendar_range::api_get_meetings_in_range,
        search::api_search_meetings,
        settings::commands::api_get_model_config,
        settings::commands::api_save_model_config,
        settings::commands::api_get_api_key,
        // settings::commands::api_get_auto_generate_setting,
        // settings::commands::api_save_auto_generate_setting,
        settings::commands::api_get_transcript_config,
        settings::commands::api_save_transcript_config,
        settings::commands::api_get_transcript_api_key,
        meetings::commands::api_create_meeting,
        meetings::commands::api_delete_meeting,
        meetings::discard::api_recording_is_safe_to_discard,
        meetings::commands::api_get_meeting,
        meetings::commands::api_get_meeting_metadata,
        meetings::commands::api_get_meeting_processing_mode,
        meetings::commands::api_set_meeting_processing_mode,
        meetings::commands::api_get_meeting_transcripts,
        meetings::commands::api_save_meeting_title,
        transcripts::api_save_transcript,
        transcripts::api_set_segment_text,
        transcripts::api_count_user_edited,
        meetings::commands::open_meeting_folder,
        utils::open_external_url,
        // Custom OpenAI commands
        settings::custom_openai::api_save_custom_openai_config,
        settings::custom_openai::api_get_custom_openai_config,
        settings::custom_openai::api_test_custom_openai_connection,
        // Summary commands
        summary::commands::api_process_transcript,
        // Day Agenda one-click "Summarize" — (re)generate from the saved
        // transcript by meeting_id, no transcript/model from frontend (specs/0012)
        summary::commands::api_generate_summary_for_meeting,
        summary::commands::api_get_summary,
        summary::commands::api_save_meeting_summary,
        summary::commands::api_get_meeting_summary_language,
        summary::commands::api_save_meeting_summary_language,
        summary::commands::api_get_meeting_detected_summary_language,
        summary::commands::api_save_meeting_detected_summary_language,
        summary::commands::api_detect_transcript_summary_language,
        summary::commands::api_cancel_summary,
        // Cross-meeting Ask-AI (specs/0035): run + cancel. Progress/result
        // arrive via ask-ai-progress / -complete / -error events.
        aggregation::commands::api_ask_ai_run,
        aggregation::commands::api_cancel_ask_ai,
        // Ask-AI history + saved questions (specs/0038 WS2.a/WS2.b): each completed run is
        // persisted fire-and-forget; these expose the history list/delete and saved-question
        // CRUD (re-run re-invokes api_ask_ai_run with the stored question+scope).
        aggregation::commands::api_list_ask_ai_history,
        aggregation::commands::api_delete_ask_ai_history,
        aggregation::commands::api_create_saved_question,
        aggregation::commands::api_list_saved_questions,
        aggregation::commands::api_delete_saved_question,
        aggregation::commands::api_get_saved_question,
        aggregation::commands::api_update_saved_question_answer,
        // Pre-call prep (specs/0036): Prep tab read, scheduled-meeting mint, brief
        // regeneration, prep-notes autosave. Brief progress arrives via prep-brief-*
        // events; prep-briefs-updated broadcasts background refreshes.
        aggregation::prep_commands::api_get_prep,
        aggregation::prep_commands::api_ensure_scheduled_meeting,
        aggregation::prep_commands::api_regenerate_prep_brief,
        aggregation::prep_commands::api_save_prep_notes,
        aggregation::prep_commands::api_get_prep_notes,
        // Manual series association (specs/0041 WS4): "Link previous meeting…" pins a
        // past recording into this meeting's series so prep briefs can find it.
        aggregation::prep_commands::api_link_meeting_to_series,
        aggregation::prep_commands::api_unlink_meeting_from_series,
        // Person roll-up (specs/0038 WS5.b): on-demand "recent themes / open
        // threads with {person}" synthesis + the plain recent-meetings list.
        aggregation::rollup::api_person_rollup,
        aggregation::rollup::api_recent_meetings_with_person,
        // Template commands
        summary::template_commands::api_list_templates,
        summary::template_commands::api_get_template_details,
        summary::template_commands::api_validate_template,
        summary::template_commands::api_get_meeting_template,
        summary::template_commands::api_set_meeting_template,
        summary::template_commands::api_save_template,
        summary::template_commands::api_delete_template,
        summary::template_commands::api_set_template_hidden,
        summary::template_commands::api_suggest_template_for_title,
        summary::template_commands::api_clear_summary_outline,
        // Built-in AI commands
        summary::summary_engine::commands::builtin_ai_list_models,
        summary::summary_engine::commands::builtin_ai_get_model_info,
        summary::summary_engine::commands::builtin_ai_download_model,
        summary::summary_engine::commands::builtin_ai_cancel_download,
        summary::summary_engine::commands::builtin_ai_delete_model,
        summary::summary_engine::commands::builtin_ai_is_model_ready,
        summary::summary_engine::commands::builtin_ai_get_available_summary_model,
        summary::summary_engine::commands::builtin_ai_get_recommended_model,
        openrouter::get_openrouter_models,
        // Recording preferences + audio backend selection
        audio::recording_preferences::get_recording_preferences,
        audio::recording_preferences::set_recording_preferences,
        audio::recording_preferences::get_default_recordings_folder_path,
        audio::recording_preferences::open_recordings_folder,
        audio::recording_preferences::select_recording_folder,
        // Audio retention (specs/0029 WS7.1): lets the UI probe whether a meeting
        // still has audio on disk (vs removed by the retention sweep).
        audio::retention::api_meeting_audio_available,
        // low-power-mode spec §5: deferred-backlog query for meetings still
        // awaiting processing with audio still on disk.
        audio::deferred_backlog::api_list_deferred_meetings,
        // Language preference commands
        audio::capture_commands::set_language_preference,
        // Notification system commands (specs/0068 — the surface macOS answers).
        notifications::os_commands::notif_capability,
        notifications::os_commands::notif_authorization_status,
        notifications::os_commands::notif_request_authorization,
        notifications::os_commands::notif_deliver,
        notifications::os_commands::notif_open_system_settings,
        // System audio capture commands
        audio::system_audio_commands::start_system_audio_capture_command,
        audio::system_audio_commands::list_system_audio_devices_command,
        audio::system_audio_commands::check_system_audio_permissions_command,
        audio::system_audio_commands::start_system_audio_monitoring,
        audio::system_audio_commands::stop_system_audio_monitoring,
        audio::system_audio_commands::get_system_audio_monitoring_status,
        // Audio Capture permission commands
        audio::permissions::check_audio_capture_permission_command,
        audio::permissions::request_audio_capture_permission_command,
        audio::permissions::trigger_system_audio_permission_command,
        // Database import commands
        database::commands::check_first_launch,
        database::commands::select_legacy_database_path,
        database::commands::detect_legacy_database,
        database::commands::check_default_legacy_database,
        database::commands::check_homebrew_database,
        database::commands::import_and_initialize_database,
        database::commands::initialize_fresh_database,
        // Database and Models path commands
        database::commands::get_database_directory,
        database::commands::open_database_folder,
        database::commands::api_save_meeting_notes,
        database::commands::api_get_meeting_notes,
        whisper_engine::commands::open_models_folder,
        // Onboarding commands
        onboarding::get_onboarding_status,
        onboarding::save_onboarding_status_cmd,
        onboarding::reset_onboarding_status_cmd,
        onboarding::complete_onboarding,
        onboarding_disk::get_models_disk_check,
        // specs/0059 — debug builds only
        #[cfg(debug_assertions)]
        dev_fixtures::commands::dev_load_fixtures,
        #[cfg(debug_assertions)]
        dev_fixtures::commands::dev_reset_onboarding,
        #[cfg(debug_assertions)]
        dev_fixtures::commands::dev_get_flags,
        #[cfg(debug_assertions)]
        dev_fixtures::commands::dev_shot_ping,
        // System settings commands
        #[cfg(target_os = "macos")]
        utils::open_system_settings,
        // Retranscription commands
        audio::retranscription::start_retranscription_command,
        audio::retranscription::cancel_retranscription_command,
        audio::retranscription::is_retranscription_in_progress_command,
        // Import audio commands
        audio::import::select_and_validate_audio_command,
        audio::import::validate_audio_file_command,
        audio::import::start_import_audio_command,
        audio::import::cancel_import_command,
        audio::import::is_import_in_progress_command,
        // Zoom auto-detection setting commands (specs/0008 P1)
        zoom::commands::api_get_zoom_auto_detect,
        zoom::commands::api_set_zoom_auto_detect,
        zoom::commands::api_get_zoom_mute_gate,
        zoom::commands::api_set_zoom_mute_gate,
        zoom::commands::api_zoom_mute_ax_trusted,
        zoom::commands::api_open_accessibility_settings,
        // In-app updates (specs/0058)
        updater::commands::api_get_update_status,
        updater::commands::api_check_for_updates,
        updater::commands::api_install_update,
        updater::commands::api_get_updater_settings,
        updater::commands::api_set_updater_settings,
        updater::commands::api_take_update_receipt,
        // macOS calendar (EventKit) commands (specs/0008 P2)
        calendar::commands::api_get_calendar_access_status,
        calendar::commands::api_request_calendar_access,
        calendar::commands::api_get_upcoming_meetings,
        // Day Agenda — unified whole-day calendar + recordings list (specs/0012)
        calendar::day_agenda::api_get_day_agenda,
        // Dismiss / ignore non-meeting calendar events (specs/0026)
        calendar::day_agenda::api_dismiss_calendar_event,
        calendar::day_agenda::api_undismiss_calendar_event,
        calendar::day_agenda::api_list_dismissed_calendar_events,
        // Google Calendar provider (specs/0032, ADR-0010)
        calendar::google::commands::api_google_calendar_status,
        calendar::google::commands::api_google_calendar_connect,
        calendar::google::commands::api_google_calendar_disconnect,
        calendar::google::commands::api_google_calendar_set_calendar_selected,
        calendar::google::commands::api_google_calendar_set_calendars_selected,
        calendar::google::commands::api_google_calendar_sync_now,
        calendar::google::commands::api_google_capabilities,
        // Speaker diarization commands (specs/0010 P1)
        diarization::commands::api_diarize_meeting,
        // Per-meeting run status for UI rehydration (specs/0029 WS3.1)
        diarization::commands::api_diarization_status,
        diarization::commands::api_get_meeting_speakers,
        diarization::model_commands::api_download_diarization_models,
        diarization::model_commands::api_diarization_models_present,
        diarization::commands::api_get_diarization_enabled,
        diarization::commands::api_set_diarization_enabled,
        // Live diarization sub-toggle (specs/0011 P3-B)
        diarization::commands::api_get_live_diarization_enabled,
        diarization::commands::api_set_live_diarization_enabled,
        // Expected-speaker-count override (specs/0011 accuracy gate)
        // Speaker labeling + calendar association (specs/0010 P2)
        diarization::commands::api_rename_speaker,
        diarization::commands::api_merge_speakers,
        diarization::corrections::api_set_segment_speaker,
        diarization::corrections::api_clear_segment_speaker,
        // Span-level manual correction + new-speaker mint (specs/0039 WS2)
        diarization::corrections::api_set_segment_speakers,
        diarization::corrections::api_create_meeting_speaker,
        // Owner-always-assignable + empty-speaker pruning (specs/0061 W4)
        diarization::speaker_maintenance::api_prune_empty_speakers,
        diarization::speaker_maintenance::api_first_segment_for_speaker,
        diarization::commands::api_get_meeting_attendees,
        diarization::commands::api_assign_speaker_to_attendee,
        diarization::commands::api_get_speaker_suggestions,
        // Persistent meeting participant roster (specs/0017 Phase A)
        diarization::commands::api_get_meeting_participants,
        diarization::commands::api_add_meeting_participant,
        diarization::commands::api_remove_meeting_participant,
        // Voiceprint gallery + consent controls (specs/0016 1c, ADR-0007)
        diarization::commands::api_get_voiceprint_settings,
        diarization::commands::api_set_store_others_voiceprints,
        diarization::commands::api_set_self_enroll_voiceprint,
        diarization::commands::api_clear_all_voiceprints,
        diarization::commands::api_get_person_voiceprint_count,
        // Per-sample voiceprint controls + retraction undo (specs/0039 WS3)
        diarization::commands::api_list_person_voiceprints,
        diarization::commands::api_quarantine_voiceprint_sample,
        diarization::commands::api_restore_voiceprint_sample,
        diarization::commands::api_delete_voiceprint_sample,
        // People directory (specs/0017/0038)
        people::commands::api_list_people,
        people::commands::api_list_people_ranked,
        people::commands::api_set_person_starred,
        people::commands::api_get_person,
        people::commands::api_create_person,
        people::commands::api_update_person,
        people::commands::api_delete_person,
        people::commands::api_set_person_voiceprint_opt_out,
        people::commands::api_assign_speaker_to_person,
        people::commands::api_get_owner_emails,
        people::commands::api_add_owner_email,
        people::commands::api_remove_owner_email,
        people::commands::api_claim_participant_as_me,
        // Action items: per-meeting section + task hub + manual extraction (specs/0034)
        action_items::commands::api_get_action_items,
        action_items::commands::api_list_action_items,
        action_items::commands::api_create_action_item,
        action_items::commands::api_update_action_item,
        action_items::commands::api_set_action_item_status,
        action_items::commands::api_delete_action_item,
        action_items::commands::api_extract_action_items,
        action_items::commands::api_reorder_action_items,
        action_items::commands::api_bulk_set_action_item_status,
        // Background LLM activity indicator (specs/0052)
        llm_activity::commands::api_llm_activity_snapshot,
        llm_activity::commands::api_llm_activity_dismiss,
        llm_activity::commands::api_llm_activity_dismiss_task,
        llm_activity::commands::api_llm_activity_retry_task,
    ]
}
