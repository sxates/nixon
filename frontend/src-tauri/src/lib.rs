// Performance optimization: Conditional logging macros for hot paths
#[cfg(debug_assertions)]
macro_rules! perf_debug {
    ($($arg:tt)*) => {
        log::debug!($($arg)*)
    };
}

#[cfg(not(debug_assertions))]
macro_rules! perf_debug {
    ($($arg:tt)*) => {};
}

#[cfg(debug_assertions)]
macro_rules! perf_trace {
    ($($arg:tt)*) => {
        log::trace!($($arg)*)
    };
}

#[cfg(not(debug_assertions))]
macro_rules! perf_trace {
    ($($arg:tt)*) => {};
}

// Re-export async logging macros for external use (removed due to macro conflicts)

// Declare audio module
pub mod action_items;
pub mod aggregation;
pub mod anthropic;
pub mod api_response;
pub mod app_paths;
pub mod audio;
pub mod calendar;
pub mod config;
pub mod diagnostics;
pub mod data_migration;
pub mod database;
pub mod dev_fixtures;
pub mod diarization;
pub mod fs_guard;
pub mod groq;
pub mod llm_activity;
pub mod meeting_detect;
pub mod meetings;
pub mod notifications;
pub mod ollama;
pub mod onboarding;
pub mod onboarding_disk;
pub mod openai;
pub mod openrouter;
pub mod parakeet_engine;
pub mod people;
pub mod power;
pub mod registry;
pub mod search;
pub mod secrets;
pub mod settings;
pub mod state;
pub mod summary;
pub mod transcripts;
pub mod tray;
pub mod updater;
pub mod utils;
pub mod whisper_engine;
pub mod window_state;
pub mod zoom;

use log::info as log_info;
use std::sync::Arc;
use tauri::Manager;

pub fn run() {
    log::set_max_level(log::LevelFilter::Info);

    let mut builder = tauri::Builder::default();

    #[cfg(any(target_os = "macos", windows, target_os = "linux"))]
    {
        builder = builder.plugin(tauri_plugin_single_instance::init(|app, args, cwd| {
            log_info!(
                "Second app instance requested with args: {:?}, cwd: {:?}",
                args,
                cwd
            );

            tray::focus_main_window(app);
        }));

        // Global record-toggle shortcut (CmdOrCtrl+Shift+R), spec 0007.
        // Tradeoff: this is a *global* (system-wide) shortcut, so it captures
        // CmdOrCtrl+Shift+R even when Nixon is unfocused — that's the point
        // (start/stop recording without bringing the app forward), but it can
        // clash with the same combo in another app. Chosen per spec 0007 for
        // unfocused capture; acceptable for v1, could become user-configurable
        // later. The actual shortcut is registered in `.setup()` below; the tray
        // accelerator in tray.rs is only a display hint, not the captor.
        use tauri::Emitter;
        use tauri_plugin_global_shortcut::ShortcutState;
        builder = builder.plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, _shortcut, event| {
                    // Debounce to the Pressed state so it fires once per press,
                    // not again on release.
                    if event.state() == ShortcutState::Pressed {
                        log::info!("Global shortcut CmdOrCtrl+Shift+R pressed: toggling recording");
                        // Emit the same event the tray + frontend already use, so
                        // it works from any route (incl. the dashboard).
                        if let Some(window) = app.get_webview_window("main") {
                            if let Err(e) = window.emit("request-recording-toggle", ()) {
                                log::error!(
                                    "Failed to emit request-recording-toggle from global shortcut: {}",
                                    e
                                );
                            }
                        } else {
                            log::warn!(
                                "Global shortcut fired but no main window found to receive request-recording-toggle"
                            );
                        }
                    }
                })
                .build(),
        );

        // specs/0058 — in-app updates. Desktop-only like the plugins above; the
        // driver's `updater_builder()` needs this plugin's state to be registered.
        builder = builder.plugin(tauri_plugin_updater::Builder::new().build());

        if let Some(plugin) = window_state::plugin() {
            builder = builder.plugin(plugin);
        }
    }

    builder
        // Persist logs to disk so the bundled .app (launched via Finder/`open`,
        // whose stderr is discarded) is debuggable. Writes to BOTH a rotating log
        // file in the OS app-log dir (macOS: ~/Library/Logs/<bundle-id>/Nixon.log,
        // e.g. ~/Library/Logs/ai.vinyl.app.debug/Nixon.log) AND stdout (for dev
        // runs). tauri-plugin-log installs the global `log` logger; main.rs no
        // longer calls env_logger::init() (two `log` loggers would panic).
        .plugin(
            tauri_plugin_log::Builder::new()
                .level(log::LevelFilter::Info)
                // Keep OS-integration modules verbose enough to debug.
                .level_for("app_lib::zoom", log::LevelFilter::Debug)
                .level_for("app_lib::calendar", log::LevelFilter::Debug)
                // The plugin defaults to a 40 KB file with `KeepOne`, which DISCARDS the
                // file on rotation. That is far too small to hold one recording — a single
                // session's VAD lines alone exceeded it — so by the time anyone looked, the
                // session they wanted was gone. It blocked two investigations on 2026-09-21:
                // the duration-accounting bug (specs/0071 W5) is diagnosed purely from a log
                // line, and that line had already been rotated away both times.
                .max_file_size(8_000_000)
                .rotation_strategy(tauri_plugin_log::RotationStrategy::KeepSome(3))
                .targets([
                    tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::LogDir {
                        file_name: None,
                    }),
                    tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::Stdout),
                ])
                .build(),
        )
        .plugin(tauri_plugin_store::Builder::default().build())
        .plugin(tauri_plugin_dialog::init())
        // specs/0058 — updater state (status machine; the payload is staged on disk).
        .manage(updater::UpdaterState::default())
        .manage(audio::init_system_audio_state())
        .manage(summary::summary_engine::ModelManagerState(Arc::new(tokio::sync::Mutex::new(None))))
        // Background LLM activity registry (specs/0052) — the sidebar indicator reads it.
        .manage(llm_activity::LlmActivityState(Arc::new(
            llm_activity::LlmTaskRegistry::new(),
        )))
        .setup(|_app| {
            log::info!("Application setup complete");

            // Attach the app handle so registry transitions emit `llm-activity-changed`.
            if let Some(activity) = _app.handle().try_state::<llm_activity::LlmActivityState>() {
                activity.0.attach(_app.handle().clone());
            }

            // Capture the identifier-derived app-data dir as the single source of
            // truth for all storage paths (templates, settings, model fallbacks).
            // See `app_paths` / ADR-0004 (dev/prod isolation).
            match _app.handle().path().app_data_dir() {
                Ok(dir) => {
                    // Before anything reads the DB/settings, migrate data from a
                    // previous bundle identifier (e.g. com.vinyl.dev → ai.vinyl.app)
                    // so a rebrand doesn't appear to wipe the user's meetings.
                    data_migration::run_at_startup(_app.handle(), dir.clone());
                    app_paths::init(dir);
                }
                Err(e) => log::error!("Failed to resolve app_data_dir for app_paths: {e}"),
            }

            // Capture the bundle identifier and bring up the Keychain-backed
            // secret store for API keys (spec 0030 WS1 / ADR-0009). Must happen
            // before the DB initializes so the startup key migration and all
            // key reads can see the store.
            app_paths::init_bundle_identifier(_app.config().identifier.clone());
            secrets::init_from_bundle_identifier();

            // specs/0057 Plan 2 — seed the active recordings write root from the
            // persisted `save_folder` BEFORE any writer can run (the startup
            // processing reconciliation below is one). Without this the writers fall
            // back to the filesystem probe and can disagree with what `fs_guard` and
            // the meetings commands read back.
            {
                let handle = _app.handle().clone();
                tauri::async_runtime::block_on(crate::audio::init_recordings_root(&handle));
            }

            // Initialize system tray
            if let Err(e) = tray::create_tray(_app.handle()) {
                log::error!("Failed to create system tray: {}", e);
            }

            // Register the process-lifetime emitter for the live recording UI events
            // (`recording-level` / `recording-spectrum`). Registered HERE — at app
            // setup — rather than from the transcription worker's startup, because
            // record-only mode (specs/0029 WS7.2, live transcription off) never spawns
            // that worker and the spectrometer/level meter must still animate. The
            // pipeline emits these from its raw mixed-window path (specs/0029 WS4.2).
            {
                use tauri::Emitter;
                let emitter_app = _app.handle().clone();
                audio::pipeline::register_live_event_emitter(move |event, payload| {
                    let _ = emitter_app.emit(event, payload);
                });
            }

            // Register the global record-toggle shortcut (CmdOrCtrl+Shift+R), spec 0007.
            // Registration can fail if the combo is already claimed system-wide
            // by another app — log and continue rather than panic.
            #[cfg(any(target_os = "macos", windows, target_os = "linux"))]
            {
                use tauri_plugin_global_shortcut::GlobalShortcutExt;
                match _app.global_shortcut().register("CmdOrCtrl+Shift+R") {
                    Ok(_) => log::info!("Registered global shortcut CmdOrCtrl+Shift+R for recording toggle"),
                    Err(e) => log::warn!(
                        "Could not register global shortcut CmdOrCtrl+Shift+R (likely already taken by another app): {}",
                        e
                    ),
                }
            }

            // Install the UNUserNotificationCenter delegate + categories (specs/0068).
            // Must happen before the first banner: the delegate is how a press gets back
            // into Nixon, and the categories are what give the banner its buttons. No-ops
            // on an unbundled dev binary, where the framework would abort the process.
            if let Err(e) = notifications::macos::install(_app.handle()) {
                log::warn!("Could not install the notification delegate: {}", e);
            }

            // Meeting auto-detection (specs/0008 P1; Teams + Meet in specs/0074 W6): emits
            // `meeting-detected` / `meeting-ended`; the frontend owns record start/stop.
            meeting_detect::spawn_meeting_monitor(_app.handle().clone());

            // Zoom mute gate (specs/0049): while recording with the opt-in setting on,
            // poll Zoom's mute state via Accessibility and drop the owner mic while muted.
            zoom::spawn_zoom_mute_monitor(_app.handle().clone());

            // specs/0058 — unattended update check + background download. Never
            // restarts by itself; dev builds set NIXON_DISABLE_UPDATER=1.
            updater::spawn_update_loop(_app.handle().clone());

            // Set models directory to use app_data_dir (unified storage location)
            whisper_engine::commands::set_models_directory(_app.handle());

            // Initialize Whisper engine on startup
            tauri::async_runtime::spawn(async {
                if let Err(e) = whisper_engine::commands::whisper_init().await {
                    log::error!("Failed to initialize Whisper engine on startup: {}", e);
                }
            });

            // Set Parakeet models directory
            parakeet_engine::commands::set_models_directory(_app.handle());

            // Initialize Parakeet engine on startup
            tauri::async_runtime::spawn(async {
                if let Err(e) = parakeet_engine::commands::parakeet_init().await {
                    log::error!("Failed to initialize Parakeet engine on startup: {}", e);
                }
            });

            // Initialize ModelManager for summary engine (async, non-blocking)
            let app_handle_for_model_manager = _app.handle().clone();
            tauri::async_runtime::spawn(async move {
                match summary::summary_engine::commands::init_model_manager_at_startup(&app_handle_for_model_manager).await {
                    Ok(_) => log::info!("ModelManager initialized successfully at startup"),
                    Err(e) => {
                        log::warn!("Failed to initialize ModelManager at startup: {}", e);
                        log::warn!("ModelManager will be lazy-initialized on first use");
                    }
                }
            });

            // Initialize database (handles first launch detection and conditional setup).
            // specs/0028: do NOT `.expect()` here — a DB-init failure must not abort the
            // process before any window exists. Surface the error to the user (event the
            // frontend can render + a native dialog) and keep the app alive so they can
            // read the message / retry rather than seeing a silent crash.
            if let Err(e) = tauri::async_runtime::block_on(async {
                database::setup::initialize_database_on_startup(_app.handle()).await
            }) {
                let msg = format!(
                    "Nixon could not open its database and may not save or load meetings: {}",
                    e
                );
                log::error!("{}", msg);

                // Emit for the frontend (in case the window comes up).
                {
                    use tauri::Emitter;
                    if let Err(emit_err) = _app.handle().emit("db-init-failed", msg.clone()) {
                        log::error!("Failed to emit db-init-failed event: {}", emit_err);
                    }
                }

                // Best-effort native dialog so the user sees it even if the UI can't load.
                #[cfg(desktop)]
                {
                    use tauri_plugin_dialog::{DialogExt, MessageDialogKind};
                    _app.handle()
                        .dialog()
                        .message(msg.as_str())
                        .kind(MessageDialogKind::Error)
                        .title("Nixon — Database Error")
                        .blocking_show();
                }
            }

            // specs/0059 — debug-only: reset onboarding / seed fixtures before the window shows.
            tauri::async_runtime::block_on(crate::onboarding::reset_if_requested(_app.handle()));
            #[cfg(debug_assertions)]
            crate::dev_fixtures::seed_at_startup(_app.handle());
            // specs/0060 — debug-only: loopback control listener for screenshot drivers.
            #[cfg(debug_assertions)]
            crate::dev_fixtures::control::spawn_if_requested(_app.handle().clone());

            // specs/0072: audio lifecycle — finish meetings a quit interrupted mid-processing,
            // then enforce the retention policy (+60 s, then hourly). After database init.
            audio::lifecycle::spawn(_app.handle().clone());

            // spec 0051 WS2: return meetings stranded at processing_mode='live' (a
            // stop-time handoff that never completed) to the deferred backlog. Spawned
            // AFTER database init; best-effort, retried on the next launch.
            audio::processing_reconcile::spawn_startup_reconciliation(_app.handle().clone());

            // specs/0073: finish a recordings move a quit interrupted, then gather meetings
            // left outside the recordings folder (the first launch asks first).
            audio::recordings_move::commands::spawn_startup_resume_and_gather(_app.handle().clone());

            // Google Calendar background sync timer (specs/0032, owner decision
            // 2026-07-02): every 10 minutes while the app runs, sync-if-stale.
            // Spawned AFTER database init (it reads the google_calendar_* tables);
            // an instant no-op while no account is connected or the build has no
            // baked-in client id.
            calendar::google::sync::spawn_background_sync(_app.handle().clone());

            // Pre-call-prep brief generator (specs/0036): keep briefs warm for the next
            // ~48h of recurring meetings so opening a meeting's Prep tab is instant. First
            // pass ~90 s after startup, then every 30 min; fingerprint-guarded so a pass
            // usually generates nothing.
            aggregation::prep_jobs::spawn_prep_generator(_app.handle().clone());

            // Power-source monitor (low-power-mode spec §1): emits
            // `power-source-changed` so the frontend can prompt for deferred
            // backlog processing when back on AC.
            power::spawn_power_monitor(_app.handle().clone());

            // Initialize bundled templates directory for dynamic template discovery
            log::info!("Initializing bundled templates directory...");
            if let Ok(resource_path) = _app.handle().path().resource_dir() {
                let templates_dir = resource_path.join("templates");
                log::info!("Setting bundled templates directory to: {:?}", templates_dir);
                summary::templates::set_bundled_templates_dir(templates_dir);
            } else {
                log::warn!("Failed to resolve resource directory for templates");
            }

            Ok(())
        })
        .on_window_event(|window, event| {
            match event {
                tauri::WindowEvent::CloseRequested { api, .. } => {
                    if window.label() == "main" {
                        api.prevent_close();
                        if let Err(e) = window.hide() {
                            log::error!("Failed to hide main window on close request: {}", e);
                        } else {
                            log::info!("Main window hidden to tray on close request");
                        }
                    }
                }
                // App-focus sync trigger for Google Calendar (specs/0032): a
                // staleness-gated, single-flighted background pass — an instant
                // no-op when not connected. Spawned so window focus never blocks.
                tauri::WindowEvent::Focused(true) if window.label() == "main" => {
                    let app = window.app_handle().clone();
                    tauri::async_runtime::spawn(async move {
                        use calendar::google::sync::{sync_if_stale, SyncTrigger};
                        sync_if_stale(&app, SyncTrigger::Focus).await;
                    });
                }
                _ => {}
            }
        })
        .invoke_handler(registry::invoke_handler())
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|_app_handle, event| {
            match event {
                #[cfg(target_os = "macos")]
                tauri::RunEvent::Reopen { .. } => {
                    tray::focus_main_window(_app_handle);
                }
                tauri::RunEvent::Exit => {
                    log::info!("Application exiting, cleaning up resources...");
                    // specs/0060: best-effort so a stale dev-control.port never survives
                    // a killed/crashed dev session into the next one.
                    #[cfg(debug_assertions)]
                    crate::dev_fixtures::control::cleanup_port_file();
                    tauri::async_runtime::block_on(async {
                        // Clean up database connection and checkpoint WAL
                        if let Some(app_state) = _app_handle.try_state::<state::AppState>() {
                            log::info!("Starting database cleanup...");
                            if let Err(e) = app_state.db_manager.cleanup().await {
                                log::error!("Failed to cleanup database: {}", e);
                            } else {
                                log::info!("Database cleanup completed successfully");
                            }
                        } else {
                            log::warn!("AppState not available for database cleanup (likely first launch)");
                        }

                        // Clean up sidecar
                        log::info!("Cleaning up sidecar...");
                        if let Err(e) = summary::summary_engine::force_shutdown_sidecar().await {
                            log::error!("Failed to force shutdown sidecar: {}", e);
                        }
                    });
                    log::info!("Application cleanup complete");
                }
                _ => {}
            }
        });
}
