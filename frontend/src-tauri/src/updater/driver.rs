//! specs/0058 — the only file that touches `tauri_plugin_updater`.
//!
//! One check = fetch `latest.json`, and if it offers a strictly newer version than what
//! is staged, stream the tarball to `<app-data-dir>/updates/`, let the plugin verify the
//! minisign signature, and mark Ready. Install re-hashes the staged file against the
//! digest taken of those verified bytes, then extracts and restarts — offline.

use std::path::PathBuf;
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, Runtime};
use tauri_plugin_updater::UpdaterExt;

use super::{emit_status, verify, InstallRefusal, UpdateStatus, UpdaterState};

const HTTP_TIMEOUT: Duration = Duration::from_secs(30);
/// Fallback emit cadence while the server sends no Content-Length (no percent to track).
const PROGRESS_EMIT_BYTES: u64 = 256 * 1024;

fn staging_dir() -> PathBuf {
    crate::app_paths::app_data_dir().join("updates")
}

fn staged_path(version: &str) -> PathBuf {
    staging_dir().join(format!("Nixon-{version}.app.tar.gz"))
}

/// Forget the staged payload and tell everyone. Used when the file it names is gone from
/// disk or the manifest has moved on: without this the core keeps believing the version
/// is staged, `should_download` keeps returning false for it, and it is never
/// re-downloaded.
fn forget_staged<R: Runtime>(app: &AppHandle<R>, why: &str) {
    log::warn!("updater: forgetting the staged payload ({why})");
    {
        let state = app.state::<UpdaterState>();
        state.core.lock().unwrap().clear_staged(chrono::Utc::now());
        // The handle describes a payload we no longer have; keeping it would let a
        // later install run against bytes that are gone or superseded.
        *state.pending.lock().unwrap() = None;
    }
    emit_status(app);
    crate::tray::update_tray_menu(app);
}

/// Should this chunk produce an `update-status` event, and what does the caller
/// remember for next time? Returns `(emit, last_pct, last_emit_bytes)`.
///
/// A ~100 MB payload arrives in thousands of chunks; one event each would flood the
/// webview. With a Content-Length we emit only when the integer percent changes, and
/// without one every `PROGRESS_EMIT_BYTES`.
fn should_emit(
    received: u64,
    content_length: Option<u64>,
    last_pct: u64,
    last_emit_bytes: u64,
) -> (bool, u64, u64) {
    match content_length {
        Some(len) if len > 0 => {
            let pct = received.saturating_mul(100) / len;
            (pct != last_pct, pct, last_emit_bytes)
        }
        _ => {
            if received.saturating_sub(last_emit_bytes) >= PROGRESS_EMIT_BYTES {
                (true, last_pct, received)
            } else {
                (false, last_pct, last_emit_bytes)
            }
        }
    }
}

/// Run one check (+ download when something newer is offered). Errors are folded into
/// the status; this never panics and never restarts the app.
pub async fn check_and_download<R: Runtime>(app: &AppHandle<R>) {
    let state = app.state::<UpdaterState>();
    // Single-flight: the loop, a manual check and the preference-toggle check can all
    // fire at once. A second concurrent run would interleave progress counters and its
    // staging-dir wipe would delete the first run's tarball while `core` still says
    // Ready, so the loser does nothing at all — the caller reads the live status.
    let Some(_in_flight) = state.try_begin_check() else {
        log::debug!("updater: a check is already running; skipping this one");
        return;
    };
    // Self-heal before anything else: the staging record is only meaningful while its
    // file exists. If someone cleaned it out between ticks, forget it here so
    // `should_download` offers the same version again instead of skipping it forever.
    let staged_file_gone = {
        let staged = state
            .core
            .lock()
            .unwrap()
            .staged_version()
            .map(str::to_string);
        match staged {
            // On an IO error assume it is still there — never drop good state on a blip.
            Some(v) => !tokio::fs::try_exists(staged_path(&v)).await.unwrap_or(true),
            None => false,
        }
    };
    if staged_file_gone {
        forget_staged(app, "its file is no longer on disk");
    }

    {
        state.core.lock().unwrap().begin_check();
    }
    emit_status(app);

    let result = async {
        let updater = app.updater_builder().timeout(HTTP_TIMEOUT).build()?;
        let Some(update) = updater.check().await? else {
            return Ok::<_, tauri_plugin_updater::Error>(None);
        };
        let wanted = state.core.lock().unwrap().should_download(&update.version);
        if !wanted {
            return Ok(None);
        }
        let version = update.version.clone();
        let notes = update.body.clone().unwrap_or_default();
        {
            state.core.lock().unwrap().begin_download(&version);
        }
        emit_status(app);

        let app_for_progress = app.clone();
        // The byte counter advances on every chunk, but the IPC event does not — see
        // `should_emit`.
        let mut last_pct = u64::MAX;
        let mut last_emit_bytes = 0u64;
        // tauri-plugin-updater 2.11: `download` verifies the minisign signature against
        // the configured pubkey BEFORE returning the bytes, so what we stage on disk is
        // already verified and `install` never sees unverified bytes.
        let bytes = update
            .download(
                move |chunk, total| {
                    let st = app_for_progress.state::<UpdaterState>();
                    let (received, content_length) = {
                        let mut core = st.core.lock().unwrap();
                        core.progress(chunk as u64, total);
                        match core.status() {
                            UpdateStatus::Downloading {
                                received, total, ..
                            } => (*received, *total),
                            _ => return,
                        }
                    };
                    let emit;
                    (emit, last_pct, last_emit_bytes) =
                        should_emit(received, content_length, last_pct, last_emit_bytes);
                    if emit {
                        emit_status(&app_for_progress);
                    }
                },
                || {},
            )
            .await?;

        let dir = staging_dir();
        tokio::fs::create_dir_all(&dir)
            .await
            .map_err(tauri_plugin_updater::Error::Io)?;
        // Drop any older staged payloads so the dir never accumulates.
        if let Ok(mut rd) = tokio::fs::read_dir(&dir).await {
            while let Ok(Some(entry)) = rd.next_entry().await {
                let _ = tokio::fs::remove_file(entry.path()).await;
            }
        }
        // Hash the bytes the plugin just signature-verified, *before* they leave memory.
        // Install re-hashes the file and refuses anything that does not match, so the
        // round trip through the staging dir can't smuggle in different bytes.
        let digest = verify::digest(&bytes);
        tokio::fs::write(staged_path(&version), &bytes)
            .await
            .map_err(tauri_plugin_updater::Error::Io)?;
        Ok(Some((version, notes, digest, update)))
    }
    .await;

    let now = chrono::Utc::now();
    {
        let mut core = state.core.lock().unwrap();
        match result {
            Ok(Some((version, notes, digest, update))) => {
                log::info!("updater: {version} downloaded and verified; waiting for restart");
                core.ready(&version, &notes, digest, now);
                // Keep the handle: install uses it instead of a fresh network check.
                *state.pending.lock().unwrap() = Some(update);
            }
            Ok(None) => core.found_none(now),
            Err(e) => {
                log::warn!("updater: check failed: {e}");
                core.failed(&e.to_string(), now);
            }
        }
    }
    emit_status(app);
    crate::tray::update_tray_menu(app);
}

/// Payload of `update-install-refused`.
#[derive(Clone, Serialize)]
struct InstallRefused {
    message: String,
}

/// Install the staged payload and relaunch.
///
/// Every refusal is broadcast as `update-install-refused` as well as returned, because
/// the tray offers this too and a tray click has nowhere to show a returned `Err`.
pub async fn install_and_restart<R: Runtime>(app: AppHandle<R>) -> Result<(), String> {
    let result = install_staged(&app).await;
    if let Err(message) = &result {
        log::warn!("updater: install refused: {message}");
        let payload = InstallRefused {
            message: message.clone(),
        };
        if let Err(e) = app.emit(super::EVENT_INSTALL_REFUSED, payload) {
            log::debug!("updater: refusal emit failed: {e}");
        }
    }
    result
}

/// The install itself. Entirely offline: the bytes on disk were signature-verified when
/// they were downloaded, we re-hash them against the digest recorded at that moment, and
/// the plugin handle kept in `UpdaterState::pending` supplies the extraction context. No
/// `check()` here — a network call could hand back a different release than the one
/// staged, and would fail outright on a plane.
async fn install_staged<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    // Resolve the recorder state *before* taking the state handle so no lock or
    // `State` guard is held across an await.
    let stopped = !crate::audio::recording_commands::is_recording().await;
    let (version, digest) = {
        let state = app.state::<UpdaterState>();
        let core = state.core.lock().unwrap();
        let version = core.can_install(stopped).map_err(|e| e.to_string())?;
        let digest = core
            .staged_digest()
            .ok_or_else(|| InstallRefusal::NothingStaged.to_string())?;
        (version, digest)
    };
    let path = staged_path(&version);
    let bytes = match tokio::fs::read(&path).await {
        Ok(bytes) => bytes,
        Err(e) => {
            forget_staged(app, "its file is missing at install time");
            return Err(format!(
                "Staged update is missing ({e}); it will be downloaded again"
            ));
        }
    };
    if !verify::matches(&bytes, &digest) {
        // Truncated, swapped, or edited since we verified it. Delete it so the next
        // check re-downloads rather than tripping over the same file forever.
        forget_staged(app, "the staged file no longer matches its verified digest");
        let _ = tokio::fs::remove_file(&path).await;
        return Err("Staged update failed verification; it will be downloaded again".to_string());
    }
    let handle = app.state::<UpdaterState>().pending.lock().unwrap().take();
    let Some(update) = handle else {
        forget_staged(app, "the plugin handle for it is gone");
        return Err("Staged update is missing; it will be downloaded again".to_string());
    };
    // The gate above was evaluated before the staged file was read and hashed, which for
    // a ~100 MB payload is not instant. Zoom auto-detect or the global record shortcut
    // can have started a recording in that window, so re-check immediately before the
    // point of no return — and put the handle back, since the payload is still good.
    if crate::audio::recording_commands::is_recording().await {
        *app.state::<UpdaterState>().pending.lock().unwrap() = Some(update);
        return Err(InstallRefusal::RecordingInProgress.to_string());
    }
    // Extraction is synchronous filesystem work; keep it off the async runtime.
    tauri::async_runtime::spawn_blocking(move || update.install(bytes))
        .await
        .map_err(|e| format!("Update install task failed: {e}"))?
        .map_err(|e| e.to_string())?;
    // specs/0069 W6 — leave the incoming version a note saying what it is. Best-effort:
    // a failed write costs the "what's new" dialog, never the install the user just asked
    // for. Scoped so the lock is released before the process is replaced.
    //
    // Review finding, fix round 2: `update.install(bytes)` above has already replaced the
    // installed bundle, and `app.restart()` below is the point of no return — an `.unwrap()`
    // on a poisoned mutex here would panic in that exact window, turning a missing "what's
    // new" dialog into a lost install. `.ok()` keeps the same "best-effort" contract: a
    // poisoned lock just means no notes, same as a missing `staged_notes()`.
    {
        let notes = app
            .state::<UpdaterState>()
            .core
            .lock()
            .ok()
            .and_then(|core| core.staged_notes().map(str::to_string))
            .unwrap_or_default();
        let receipt = super::receipt::UpdateReceipt {
            version: version.clone(),
            notes,
            installed_at: chrono::Utc::now(),
        };
        if let Err(e) = super::receipt::write(&receipt) {
            log::warn!("updater: could not record the update receipt (continuing): {e}");
        }
    }
    log::info!("updater: installed {version}; restarting");
    crate::window_state::save(app);
    app.restart();
}

#[cfg(test)]
mod tests {
    use super::*;

    const MB: u64 = 1024 * 1024;

    #[test]
    fn first_chunk_always_emits() {
        assert!(should_emit(1, Some(100 * MB), u64::MAX, 0).0);
        assert!(
            !should_emit(1, None, u64::MAX, 0).0,
            "with no Content-Length the first chunk waits for the byte cadence"
        );
    }

    #[test]
    fn with_a_content_length_only_a_percent_change_emits() {
        // 100-byte payload: 41 -> 41% (new), 41 again (same percent), 42 -> 42%.
        let (emit, pct, bytes) = should_emit(41, Some(100), 40, 0);
        assert!(emit);
        assert_eq!((pct, bytes), (41, 0));

        let (emit, pct, _) = should_emit(41, Some(100), pct, bytes);
        assert!(!emit, "the same integer percent must not emit again");
        assert_eq!(pct, 41);

        let (emit, pct, _) = should_emit(42, Some(100), pct, bytes);
        assert!(emit);
        assert_eq!(pct, 42);
    }

    #[test]
    fn a_zero_content_length_falls_back_to_the_byte_cadence() {
        assert!(!should_emit(1024, Some(0), u64::MAX, 0).0);
        assert!(should_emit(PROGRESS_EMIT_BYTES, Some(0), u64::MAX, 0).0);
    }

    #[test]
    fn without_a_content_length_it_emits_every_256_kb() {
        let (emit, _, bytes) = should_emit(PROGRESS_EMIT_BYTES - 1, None, u64::MAX, 0);
        assert!(!emit, "under the cadence, stay quiet");
        assert_eq!(bytes, 0, "and remember nothing");

        let (emit, _, bytes) = should_emit(PROGRESS_EMIT_BYTES, None, u64::MAX, bytes);
        assert!(emit);
        assert_eq!(
            bytes, PROGRESS_EMIT_BYTES,
            "the mark moves to where we emitted"
        );

        let (emit, _, _) = should_emit(PROGRESS_EMIT_BYTES + 1, None, u64::MAX, bytes);
        assert!(!emit, "one byte later is not another 256 KB");

        let (emit, _, bytes) = should_emit(3 * PROGRESS_EMIT_BYTES, None, u64::MAX, bytes);
        assert!(emit, "a jumbo chunk that skips a whole step still emits");
        assert_eq!(bytes, 3 * PROGRESS_EMIT_BYTES);
    }
}
