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
    /// The user picked this `save_folder` by hand (Settings → Recording → Change…), as
    /// opposed to it having come from the probe.
    ///
    /// Exists only so the debug re-point in [`init_recordings_root`] can tell "nobody ever
    /// chose this" from "a developer pointed this build somewhere on purpose". The store
    /// holds only the resulting path, so the intent is not otherwise recoverable.
    ///
    /// `#[serde(default)]` means every existing preferences file reads as "not chosen",
    /// which is the safe answer for a debug profile and irrelevant in a release build,
    /// where the re-point never runs.
    #[serde(default)]
    pub save_folder_user_chosen: bool,
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
            save_folder_user_chosen: false,
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

/// Folder name for the DEBUG build ("Dev Nixon", identifier `ai.vinyl.app.debug`).
///
/// ADR-0004 isolates dev from production in every other respect — separate SQLite DB,
/// settings, single-instance lock, TCC permissions — but the recordings root was resolved by
/// the same probe for both, so on a machine with a pre-0057 install the dev build wrote into
/// `~/Movies/meetily-recordings` alongside the owner's real meetings. Which is how the
/// owner's actual home path, under the *fork's* name, ended up legible in a screenshot
/// committed to a public repo (owner feedback 2026-09-21).
const DEV_RECORDINGS_DIR: &str = "nixon-recordings-dev";

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
///
/// In a DEBUG build this also re-points a `save_folder` that was never chosen by hand —
/// see [`repoint_dev_root`].
pub async fn init_recordings_root<R: Runtime>(app: &AppHandle<R>) {
    match load_recording_preferences(app).await {
        Ok(prefs) => {
            let prefs = repoint_dev_root(app, prefs).await;
            set_recordings_root(prefs.save_folder);
        }
        Err(e) => {
            warn!("recordings root: could not load preferences, using default probe: {e}")
        }
    }
}

/// Move a debug build off a production recordings folder, once.
///
/// Changing [`default_recordings_folder_for_profile`] alone moves nothing on a machine that
/// has already run Nixon: the persisted `save_folder` is authoritative (specs/0057 Plan 2),
/// and every debug profile that existed before this change has the legacy path stored in it.
///
/// Three things make this safe to do automatically:
///
///  - It is gated on [`is_dev_build`]. A release build never reaches it, so no user's
///    recordings folder is ever moved out from under them.
///  - It respects `save_folder_user_chosen`. A developer who deliberately pointed the dev
///    build at a folder keeps it.
///  - It PERSISTS through [`save_recording_preferences`] rather than only seeding the cache.
///    `fs_guard::allowed_fs_roots` re-reads the store on every call, so a cache-only change
///    would leave the webview's allow-list pointing at the old root while writes went to the
///    new one.
///
/// Existing dev recordings are not moved. They stay readable because `fs_guard` also
/// allow-lists the release root in debug builds.
async fn repoint_dev_root<R: Runtime>(
    app: &AppHandle<R>,
    prefs: RecordingPreferences,
) -> RecordingPreferences {
    if !is_dev_build() || prefs.save_folder_user_chosen {
        return prefs;
    }
    let wanted = get_default_recordings_folder();
    if prefs.save_folder == wanted {
        return prefs;
    }
    info!(
        "recordings root: dev build re-pointed from {:?} to {:?} (ADR-0004 isolation)",
        prefs.save_folder, wanted
    );
    let mut next = prefs;
    next.save_folder = wanted;
    if let Err(e) = save_recording_preferences(app, &next).await {
        // Persisting is the whole point — without it `fs_guard` and the write root
        // disagree. Fall back to the stored value rather than run in that split state.
        warn!("recordings root: could not persist the dev re-point ({e}); keeping the stored folder");
        return load_recording_preferences(app).await.unwrap_or(next);
    }
    next
}

/// Resolve the default recordings folder for a given build profile.
///
/// `dev` short-circuits the monotone legacy/new probe entirely. That probe exists to protect
/// a production install whose persisted `save_folder` already points at the fork's folder
/// (specs/0057 Plan 2 — flipping the write root would orphan recordings). The debug profile
/// has no such data to protect, so inheriting the rule bought it nothing and cost it the
/// isolation ADR-0004 promises.
///
/// Split from [`default_recordings_folder_in`] rather than folded into it with a `cfg!`,
/// because `cargo test` runs WITH `debug_assertions` — a `cfg!` inside the policy would make
/// the production-behaviour tests untestable, which is the sort of thing that gets noticed
/// after it ships.
fn default_recordings_folder_for_profile(base: &Path, dev: bool) -> PathBuf {
    if dev {
        return base.join(DEV_RECORDINGS_DIR);
    }
    default_recordings_folder_in(base)
}

/// Is this a debug ("Dev Nixon") build?
fn is_dev_build() -> bool {
    cfg!(debug_assertions)
}

/// Get the default recordings folder based on platform.
///
/// See [`default_recordings_folder_in`] for why an existing `meetily-recordings` folder
/// still wins in a release build, and [`default_recordings_folder_for_profile`] for why the
/// debug build opts out of that rule.
fn get_default_recordings_folder() -> PathBuf {
    default_recordings_folder_for_profile(&platform_recordings_base(), is_dev_build())
}

/// The folder a RELEASE build would resolve to, regardless of this build's profile.
///
/// Used only by `fs_guard` in debug builds, to keep pre-existing dev recordings readable
/// after the re-point below moves the write root. Never a write target.
pub(crate) fn release_default_recordings_folder() -> PathBuf {
    default_recordings_folder_for_profile(&platform_recordings_base(), false)
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

    // -- the debug build's own recordings root (owner feedback 2026-09-21) -------------
    //
    // The monotone legacy/new probe above protects a production install whose persisted
    // save_folder already points at the fork's folder. The debug profile has no such data,
    // so it opted out — which is also how the owner's real home path, under the *fork's*
    // name, ended up legible in a screenshot committed to a public repo.
    //
    // These pass `dev` explicitly rather than relying on the build flag: `cargo test` runs
    // WITH debug_assertions, so a `cfg!` inside the policy would make the production cases
    // above impossible to test.

    #[test]
    fn a_dev_build_gets_its_own_folder_on_a_fresh_machine() {
        let base = tempdir("dev-fresh");

        assert_eq!(
            default_recordings_folder_for_profile(&base, true),
            base.join(DEV_RECORDINGS_DIR)
        );

        std::fs::remove_dir_all(&base).ok();
    }

    /// The case that produced the leak: a machine carrying a pre-0057 install. The release
    /// probe must keep choosing the legacy folder, and the dev build must not.
    #[test]
    fn a_dev_build_ignores_a_legacy_folder_the_release_build_would_take() {
        let base = tempdir("dev-legacy");
        std::fs::create_dir_all(base.join(LEGACY_RECORDINGS_DIR)).unwrap();

        assert_eq!(
            default_recordings_folder_for_profile(&base, false),
            base.join(LEGACY_RECORDINGS_DIR),
            "release still honours the persisted legacy folder (specs/0057 Plan 2)"
        );
        assert_eq!(
            default_recordings_folder_for_profile(&base, true),
            base.join(DEV_RECORDINGS_DIR),
            "the dev build has no production data to protect"
        );

        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn a_dev_build_ignores_a_nixon_folder_too() {
        // Not just the legacy name: the dev root is its own folder either way, so dev and
        // production recordings can never land in the same directory.
        let base = tempdir("dev-nixon");
        std::fs::create_dir_all(base.join(RECORDINGS_DIR)).unwrap();

        assert_eq!(
            default_recordings_folder_for_profile(&base, true),
            base.join(DEV_RECORDINGS_DIR)
        );
        assert_ne!(
            default_recordings_folder_for_profile(&base, true),
            default_recordings_folder_for_profile(&base, false)
        );

        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn the_dev_folder_is_named_for_nixon_not_the_fork() {
        assert!(DEV_RECORDINGS_DIR.starts_with("nixon-"));
        assert!(!DEV_RECORDINGS_DIR.contains("meetily"));
        assert_ne!(DEV_RECORDINGS_DIR, RECORDINGS_DIR, "dev is its own folder");
    }

    /// `#[serde(default)]` on the new marker is what keeps every preferences file written
    /// before 2026-09-21 deserializing — and it must default to "not chosen", or the debug
    /// re-point would decline to run on exactly the profiles that need it.
    #[test]
    fn an_older_preferences_file_reads_as_not_user_chosen() {
        let prefs: RecordingPreferences = serde_json::from_str(
            r#"{"save_folder":"/Users/x/Movies/meetily-recordings","auto_save":true}"#,
        )
        .unwrap();
        assert!(!prefs.save_folder_user_chosen);
    }

    #[test]
    fn a_hand_picked_folder_round_trips_as_user_chosen() {
        let prefs: RecordingPreferences = serde_json::from_str(
            r#"{"save_folder":"/Volumes/Audio","auto_save":true,"save_folder_user_chosen":true}"#,
        )
        .unwrap();
        assert!(prefs.save_folder_user_chosen);
        // And survives a serialize/deserialize round trip, since that is how it is stored.
        let again: RecordingPreferences =
            serde_json::from_str(&serde_json::to_string(&prefs).unwrap()).unwrap();
        assert!(again.save_folder_user_chosen);
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
