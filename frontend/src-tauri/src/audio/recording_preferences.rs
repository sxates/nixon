use log::{info, warn};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::RwLock;
use tauri::{AppHandle, Runtime};
use tauri_plugin_store::StoreExt;

use anyhow::Result;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RecordingPreferences {
    pub save_folder: PathBuf,
    pub auto_save: bool,
    #[serde(default)]
    pub preferred_mic_device: Option<String>,
    #[serde(default)]
    pub preferred_system_device: Option<String>,
    /// Vestigial (specs/0066 W1): the System Audio Backend control is gone and nothing
    /// reads this any more. Kept so an existing preferences file still deserializes —
    /// system capture is pinned to the platform default backend (the Core Audio tap).
    #[cfg(target_os = "macos")]
    #[serde(default)]
    pub system_audio_backend: Option<String>,
    /// Auto-delete recorded audio after this many days (specs/0029 WS7.1).
    /// `None` = keep forever (the default). The background sweep in
    /// `audio::retention` removes only media files (audio.* / mic.wav /
    /// system.wav) from each meeting folder — transcripts, notes, summaries,
    /// and `metadata.json` are never touched. `#[serde(default)]` keeps
    /// preferences stored before this field existed deserializing cleanly.
    #[serde(default)]
    pub retention_days: Option<u32>,
    /// Real-time (live) transcription during recording (specs/0029 WS7.2).
    /// `true` (the default) = the classic behavior: VAD + STT run while
    /// recording and the live transcript streams in. `false` = record-only
    /// mode: audio is still captured, mixed, and saved (and the level/spectrum
    /// visualizers keep working), but the VAD/STT stage is skipped entirely to
    /// save CPU/battery; the meeting is transcribed later via the deferred
    /// path (`retranscription.rs` — "Transcribe now" or automatically before a
    /// summary). Defaulted `true` so stored preferences from before this field
    /// existed keep live transcription on.
    #[serde(default = "default_live_transcription_enabled")]
    pub live_transcription_enabled: bool,
    /// Low Power Mode (2026-07-21 spec §2): when `true` (the default) and the
    /// Mac is on battery, recordings run in record-only mode — audio saved,
    /// VAD/STT and summaries deferred until back on AC — unless the meeting's
    /// `processing_mode` override says otherwise. `#[serde(default)]` keeps
    /// previously-stored preferences deserializing (as `true`).
    #[serde(default = "default_low_power_on_battery")]
    pub low_power_on_battery: bool,
}

/// serde default for [`RecordingPreferences::live_transcription_enabled`].
fn default_live_transcription_enabled() -> bool {
    true
}

/// serde default for [`RecordingPreferences::low_power_on_battery`].
fn default_low_power_on_battery() -> bool {
    true
}


impl Default for RecordingPreferences {
    fn default() -> Self {
        Self {
            save_folder: get_default_recordings_folder(),
            auto_save: true,
            preferred_mic_device: None,
            preferred_system_device: None,
            #[cfg(target_os = "macos")]
            system_audio_backend: Some("coreaudio".to_string()),
            retention_days: None,
            live_transcription_enabled: true,
            low_power_on_battery: true,
        }
    }
}

/// The platform directory the recordings folder lives *inside*.
///
/// macOS: `~/Movies`, Windows: `%USERPROFILE%\Music`, elsewhere `~/Documents`;
/// each falls back to `~/Documents` and then to `.` when the well-known
/// directory cannot be resolved.
fn platform_recordings_base() -> PathBuf {
    #[cfg(target_os = "windows")]
    let preferred = dirs::audio_dir();

    #[cfg(target_os = "macos")]
    let preferred = dirs::video_dir();

    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let preferred: Option<PathBuf> = None;

    preferred
        .or_else(dirs::document_dir)
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Folder name used by fresh installs (specs/0057 rebrand).
const RECORDINGS_DIR: &str = "nixon-recordings";

/// Folder name inherited from the meetily fork, kept for installs that already
/// have one.
const LEGACY_RECORDINGS_DIR: &str = "meetily-recordings";

/// Resolve the default recordings folder inside `base`.
///
/// Since specs/0057 Plan 2 this is only the **fallback** for
/// [`RecordingsRootCache`]: every writer goes through [`recordings_root`], which
/// prefers the persisted `save_folder` that `fs_guard` and the meetings commands
/// read back. The probe is consulted only when no preference was ever stored
/// (first launch, unit tests). It still matters that the two agree, because an
/// install created before specs/0057 has `~/Movies/meetily-recordings` stored in its
/// preferences — a probe that renamed the default outright would, on that first
/// launch, write new recordings outside `fs_guard`'s allowed roots and orphan them.
///
/// So the probe is **monotone**: the legacy folder wins for as long as it
/// exists, even if a `nixon-recordings` folder later appears alongside it (an
/// export, a manual mkdir, a second install). A probe that flipped on the new
/// folder's appearance would silently move the write root away from the
/// persisted `save_folder`. The new name is used only when there is no legacy
/// folder at all — a fresh install. No preferences migration is performed; the
/// stored `save_folder` stays authoritative wherever it is set.
fn default_recordings_folder_in(base: &Path) -> PathBuf {
    let legacy = base.join(LEGACY_RECORDINGS_DIR);
    if legacy.is_dir() {
        return legacy;
    }
    base.join(RECORDINGS_DIR)
}

/// The ACTIVE recordings write root (specs/0057 Plan 2, Task 1).
///
/// Every writer (saver, import, recovery, reconcile, diarization) used to call
/// `get_default_recordings_folder` — a filesystem probe — while `fs_guard`, folder
/// delete and "Open recordings folder" read the PERSISTED `save_folder`. The two
/// disagreed whenever a legacy/new folder appeared on disk after the preference was
/// stored. This cache is set from the persisted preference at startup and on every
/// preference save, so writers and readers share one source of truth; the probe is only
/// the fallback when no preference was ever stored (first launch, unit tests).
#[derive(Default)]
pub(crate) struct RecordingsRootCache(RwLock<Option<PathBuf>>);

impl RecordingsRootCache {
    pub fn set(&self, root: PathBuf) {
        *self.0.write().unwrap_or_else(|e| e.into_inner()) = Some(root);
    }

    pub fn resolve_with(&self, fallback: impl FnOnce() -> PathBuf) -> PathBuf {
        match self.0.read().unwrap_or_else(|e| e.into_inner()).as_ref() {
            Some(p) => p.clone(),
            None => fallback(),
        }
    }
}

static RECORDINGS_ROOT: RecordingsRootCache = RecordingsRootCache(RwLock::new(None));

/// Set the active write root (called from [`init_recordings_root`] and
/// [`save_recording_preferences`]).
pub fn set_recordings_root(root: PathBuf) {
    RECORDINGS_ROOT.set(root);
}

/// The folder new recordings are written to. Prefer this over the module-private
/// `get_default_recordings_folder` probe.
pub fn recordings_root() -> PathBuf {
    RECORDINGS_ROOT.resolve_with(get_default_recordings_folder)
}

/// Load the persisted preference once at startup and seed the cache. Failure to load
/// leaves the probe fallback in place (logged).
pub async fn init_recordings_root<R: Runtime>(app: &AppHandle<R>) {
    match load_recording_preferences(app).await {
        Ok(prefs) => set_recordings_root(prefs.save_folder),
        Err(e) => {
            warn!("recordings root: could not load preferences, using default probe: {e}")
        }
    }
}

/// Get the default recordings folder based on platform.
///
/// See [`default_recordings_folder_in`] for why an existing
/// `meetily-recordings` folder still wins.
fn get_default_recordings_folder() -> PathBuf {
    default_recordings_folder_in(&platform_recordings_base())
}

/// Ensure the recordings directory exists
pub fn ensure_recordings_directory(path: &PathBuf) -> Result<()> {
    if !path.exists() {
        std::fs::create_dir_all(path)?;
        info!("Created recordings directory: {:?}", path);
    }
    Ok(())
}

/// Load recording preferences from store
pub async fn load_recording_preferences<R: Runtime>(
    app: &AppHandle<R>,
) -> Result<RecordingPreferences> {
    // Try to load from Tauri store
    let store = match app.store("recording_preferences.json") {
        Ok(store) => store,
        Err(e) => {
            warn!("Failed to access store: {}, using defaults", e);
            return Ok(RecordingPreferences::default());
        }
    };

    // Try to get the preferences from store
    let prefs = if let Some(value) = store.get("preferences") {
        match serde_json::from_value::<RecordingPreferences>(value.clone()) {
            Ok(p) => {
                info!("Loaded recording preferences from store");
                p
            }
            Err(e) => {
                warn!("Failed to deserialize preferences: {}, using defaults", e);
                RecordingPreferences::default()
            }
        }
    } else {
        info!("No stored preferences found, using defaults");
        RecordingPreferences::default()
    };

    info!(
        "Loaded recording preferences: save_folder={:?}, auto_save={}, mic={:?}, system={:?}",
        prefs.save_folder,
        prefs.auto_save,
        prefs.preferred_mic_device,
        prefs.preferred_system_device
    );
    Ok(prefs)
}

/// Save recording preferences to store
pub async fn save_recording_preferences<R: Runtime>(
    app: &AppHandle<R>,
    preferences: &RecordingPreferences,
) -> Result<()> {
    info!(
        "Saving recording preferences: save_folder={:?}, auto_save={}, mic={:?}, system={:?}",
        preferences.save_folder,
        preferences.auto_save,
        preferences.preferred_mic_device,
        preferences.preferred_system_device
    );

    // Get or create store
    let store = app
        .store("recording_preferences.json")
        .map_err(|e| anyhow::anyhow!("Failed to access store: {}", e))?;

    // Serialize preferences to JSON value
    let prefs_value = serde_json::to_value(preferences)
        .map_err(|e| anyhow::anyhow!("Failed to serialize preferences: {}", e))?;

    // Save to store
    store.set("preferences", prefs_value);

    // Persist to disk
    store
        .save()
        .map_err(|e| anyhow::anyhow!("Failed to save store to disk: {}", e))?;

    info!("Successfully persisted recording preferences to disk");

    // specs/0057 Plan 2 — keep the active write root in lockstep with the persisted
    // preference, so writers land where `fs_guard` and the meetings commands look.
    set_recordings_root(preferences.save_folder.clone());

    // Ensure the directory exists
    ensure_recordings_directory(&preferences.save_folder)?;

    Ok(())
}

/// Tauri commands for recording preferences
#[tauri::command]
pub async fn get_recording_preferences<R: Runtime>(
    app: AppHandle<R>,
) -> Result<RecordingPreferences, String> {
    load_recording_preferences(&app)
        .await
        .map_err(|e| format!("Failed to load recording preferences: {}", e))
}

#[tauri::command]
pub async fn set_recording_preferences<R: Runtime>(
    app: AppHandle<R>,
    preferences: RecordingPreferences,
) -> Result<(), String> {
    save_recording_preferences(&app, &preferences)
        .await
        .map_err(|e| format!("Failed to save recording preferences: {}", e))
}

#[tauri::command]
pub async fn get_default_recordings_folder_path() -> Result<String, String> {
    let path = get_default_recordings_folder();
    Ok(path.to_string_lossy().to_string())
}

#[tauri::command]
pub async fn open_recordings_folder<R: Runtime>(app: AppHandle<R>) -> Result<(), String> {
    let preferences = load_recording_preferences(&app)
        .await
        .map_err(|e| format!("Failed to load preferences: {}", e))?;

    // Ensure directory exists before trying to open it
    ensure_recordings_directory(&preferences.save_folder)
        .map_err(|e| format!("Failed to create directory: {}", e))?;

    let folder_path = preferences.save_folder.to_string_lossy().to_string();

    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer")
            .arg(&folder_path)
            .spawn()
            .map_err(|e| format!("Failed to open folder: {}", e))?;
    }

    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(&folder_path)
            .spawn()
            .map_err(|e| format!("Failed to open folder: {}", e))?;
    }

    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        std::process::Command::new("xdg-open")
            .arg(&folder_path)
            .spawn()
            .map_err(|e| format!("Failed to open folder: {}", e))?;
    }

    info!("Opened recordings folder: {}", folder_path);
    Ok(())
}

#[tauri::command]
pub async fn select_recording_folder<R: Runtime>(
    app: AppHandle<R>,
) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;

    // blocking_pick_folder opens a native, synchronous folder chooser — it must not run
    // on the async executor thread, hence spawn_blocking (specs/0061 W6).
    let picked = tokio::task::spawn_blocking(move || app.dialog().file().blocking_pick_folder())
        .await
        .map_err(|e| format!("Folder picker task panicked: {}", e))?;

    Ok(picked.map(|p| p.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// specs/0061 W6 — `file_format` was removed from [`RecordingPreferences`] (the
    /// "File format" settings row was dead: nothing let a user change it, and the
    /// value was never actually used to pick an encoding). Preferences saved to disk
    /// by an older build still have the key. `RecordingPreferences` carries no
    /// `#[serde(deny_unknown_fields)]`, so serde must silently ignore it rather than
    /// fail to load a user's existing preferences file.
    #[test]
    fn unknown_saved_field_is_ignored_on_deserialize() {
        let json = r#"{"save_folder":"/tmp/x","auto_save":true,"file_format":"mp4"}"#;
        let prefs: RecordingPreferences =
            serde_json::from_str(json).expect("unknown fields must not fail deserialization");
        assert_eq!(prefs.save_folder, PathBuf::from("/tmp/x"));
        assert!(prefs.auto_save);
    }

    fn tempdir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "nixon-0057-recdir-{tag}-{}-{:?}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn legacy_folder_wins_when_it_is_the_only_one_present() {
        // An install from before specs/0057: its persisted save_folder points at
        // meetily-recordings, so the write root must keep matching it.
        let base = tempdir("legacy-only");
        std::fs::create_dir_all(base.join(LEGACY_RECORDINGS_DIR)).unwrap();

        assert_eq!(
            default_recordings_folder_in(&base),
            base.join(LEGACY_RECORDINGS_DIR)
        );

        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn fresh_install_uses_the_nixon_folder() {
        let base = tempdir("fresh");

        assert_eq!(
            default_recordings_folder_in(&base),
            base.join(RECORDINGS_DIR)
        );

        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn legacy_folder_wins_when_both_exist() {
        // Monotone probe: once a legacy install exists, a nixon-recordings folder
        // appearing beside it must NOT move the write root off the persisted
        // save_folder (specs/0057 final review).
        let base = tempdir("both");
        std::fs::create_dir_all(base.join(LEGACY_RECORDINGS_DIR)).unwrap();
        std::fs::create_dir_all(base.join(RECORDINGS_DIR)).unwrap();

        assert_eq!(
            default_recordings_folder_in(&base),
            base.join(LEGACY_RECORDINGS_DIR)
        );

        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn a_legacy_file_is_not_mistaken_for_the_legacy_folder() {
        let base = tempdir("legacy-file");
        std::fs::write(base.join(LEGACY_RECORDINGS_DIR), b"not a directory").unwrap();

        assert_eq!(
            default_recordings_folder_in(&base),
            base.join(RECORDINGS_DIR)
        );

        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn recordings_root_falls_back_to_the_probe_when_unset() {
        // A fresh cache (nothing seeded from the persisted preference yet) must
        // defer to the filesystem probe. Each test builds its own cache, so the
        // two are order-independent and never touch the process-wide static.
        let cache = RecordingsRootCache::default();
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(
            cache.resolve_with(|| tmp.path().join("probe")),
            tmp.path().join("probe")
        );
    }

    #[test]
    fn recordings_root_prefers_the_cached_persisted_folder() {
        let cache = RecordingsRootCache::default();
        let tmp = tempfile::tempdir().unwrap();
        cache.set(tmp.path().join("chosen"));
        assert_eq!(
            cache.resolve_with(|| tmp.path().join("probe")),
            tmp.path().join("chosen")
        );
        // a later set replaces the earlier one (Settings -> change folder)
        cache.set(tmp.path().join("chosen2"));
        assert_eq!(
            cache.resolve_with(|| PathBuf::from("/never")),
            tmp.path().join("chosen2")
        );
    }
}
