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
    /// Recordings folders used before the current one that may still hold meetings
    /// (specs/0073). Backend-owned: [`set_recording_preferences`] carries the stored value
    /// over whatever the frontend sends. Feeds [`known_recording_roots`].
    #[serde(default)]
    pub previous_save_folders: Vec<PathBuf>,
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
            previous_save_folders: Vec::new(),
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

/// Cache of the persisted `previous_save_folders`, seeded alongside [`RECORDINGS_ROOT`].
static PREVIOUS_ROOTS: RwLock<Vec<PathBuf>> = RwLock::new(Vec::new());

/// Set the earlier recordings folders [`known_recording_roots`] reports.
pub fn set_previous_recording_roots(roots: Vec<PathBuf>) {
    *PREVIOUS_ROOTS.write().unwrap_or_else(|e| e.into_inner()) = roots;
}

/// Every recordings folder Nixon has written meetings to (specs/0073): the current root,
/// the persisted earlier ones, and the platform default folders (legacy `meetily-recordings`
/// and `nixon-recordings`) that exist on disk. A debug build also includes the folder a
/// release build would use (the 0070 case: dev meetings recorded before the dev root moved).
///
/// Anything that scans "the recordings root" for existing meetings scans these instead.
pub fn known_recording_roots() -> Vec<PathBuf> {
    let base = platform_recordings_base();
    let mut extras: Vec<PathBuf> = [LEGACY_RECORDINGS_DIR, RECORDINGS_DIR]
        .iter()
        .map(|name| base.join(name))
        .filter(|p| p.is_dir())
        .collect();
    if is_dev_build() {
        extras.push(release_default_recordings_folder());
    }
    let previous = PREVIOUS_ROOTS
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    known_roots_from(recordings_root(), &previous, &extras)
}

/// Pure half of [`known_recording_roots`]: `current` first, then `previous`, then `extras`,
/// canonicalized where they exist and de-duplicated.
pub fn known_roots_from(
    current: PathBuf,
    previous: &[PathBuf],
    extras: &[PathBuf],
) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    for root in std::iter::once(current)
        .chain(previous.iter().cloned())
        .chain(extras.iter().cloned())
    {
        let root = root.canonicalize().unwrap_or(root);
        if !out.contains(&root) {
            out.push(root);
        }
    }
    out
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
            set_previous_recording_roots(prefs.previous_save_folders);
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
        warn!(
            "recordings root: could not persist the dev re-point ({e}); keeping the stored folder"
        );
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
    set_previous_recording_roots(preferences.previous_save_folders.clone());

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
    let stored = load_recording_preferences(&app)
        .await
        .map_err(|e| format!("Failed to load recording preferences: {}", e))?;
    if stored.save_folder != preferences.save_folder {
        super::volume_check::ensure_recordings_volume_allowed(&preferences.save_folder)
            .map_err(|e| e.to_string())?;
    }
    save_recording_preferences(&app, &carry_backend_fields(&stored, preferences))
        .await
        .map_err(|e| format!("Failed to save recording preferences: {}", e))
}

/// Merge a frontend-sent preferences object over the stored one (specs/0073): the frontend
/// doesn't own `previous_save_folders`, so the stored list is kept, and a folder change adds
/// the outgoing folder to it so meetings left there stay known.
fn carry_backend_fields(
    stored: &RecordingPreferences,
    mut incoming: RecordingPreferences,
) -> RecordingPreferences {
    let mut previous = stored.previous_save_folders.clone();
    if stored.save_folder != incoming.save_folder && !previous.contains(&stored.save_folder) {
        previous.push(stored.save_folder.clone());
    }
    previous.retain(|p| *p != incoming.save_folder);
    incoming.previous_save_folders = previous;
    incoming
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
mod tests;
