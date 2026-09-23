// src/fs_guard.rs
//
// Webview filesystem confinement (specs/0028) and the file read/write commands
// built on it. Moved out of lib.rs (specs/0042 WS1) — not audio-specific, so it
// lives beside the other top-level modules rather than under audio/.

use log::info as log_info;
use tauri::{AppHandle, Manager, Runtime};

use crate::audio;

/// Collect the canonical app-controlled roots the webview is allowed to read/write: every
/// recordings folder Nixon has written meetings to (specs/0073 — the current one, earlier
/// ones, and in a debug build the release root, see `known_recording_roots`) and the
/// app-data directory. Paths outside all of these are rejected. (specs/0028 — privacy: no
/// arbitrary-FS access from the webview.)
fn allowed_fs_roots<R: Runtime>(app: &AppHandle<R>) -> Vec<std::path::PathBuf> {
    let mut roots = audio::recording_preferences::known_recording_roots();
    if let Ok(dir) = app.path().app_data_dir() {
        roots.push(dir.canonicalize().unwrap_or(dir));
    }
    roots
}

/// Guard a webview-supplied path: it must resolve to a location strictly inside one of
/// the app-controlled roots. Defeats `..` traversal by canonicalizing where possible;
/// for a not-yet-existing target (e.g. a file being written) we canonicalize the parent
/// directory and re-append the filename so traversal in the parent is still caught.
fn confine_to_roots(
    file_path: &str,
    roots: &[std::path::PathBuf],
) -> Result<std::path::PathBuf, String> {
    let target = std::path::Path::new(file_path);

    let canonical_target = match target.canonicalize() {
        Ok(p) => p,
        Err(_) => {
            // Path may not exist yet (write case): canonicalize the parent then re-join.
            match (target.parent(), target.file_name()) {
                (Some(parent), Some(name)) => match parent.canonicalize() {
                    Ok(p) => p.join(name),
                    Err(_) => target.to_path_buf(),
                },
                _ => target.to_path_buf(),
            }
        }
    };

    if roots.iter().any(|root| canonical_target.starts_with(root)) {
        Ok(canonical_target)
    } else {
        Err(format!(
            "Access denied: '{}' is outside the app's allowed data directories",
            file_path
        ))
    }
}

#[tauri::command]
pub async fn read_audio_file<R: Runtime>(
    app: AppHandle<R>,
    file_path: String,
) -> Result<Vec<u8>, String> {
    let roots = allowed_fs_roots(&app);
    let safe_path = confine_to_roots(&file_path, &roots)?;

    match std::fs::read(&safe_path) {
        Ok(data) => Ok(data),
        Err(e) => Err(format!("Failed to read audio file: {}", e)),
    }
}

#[tauri::command]
pub async fn save_transcript<R: Runtime>(
    app: AppHandle<R>,
    file_path: String,
    content: String,
) -> Result<(), String> {
    log_info!("Saving transcript to: {}", file_path);

    let roots = allowed_fs_roots(&app);
    let safe_path = confine_to_roots(&file_path, &roots)?;

    write_transcript(&safe_path, &content)?;
    log_info!("Transcript saved successfully");
    Ok(())
}

/// Write `content` to `path`, whose folder must already exist. A missing folder is never
/// created: a path captured before the recordings folder moved would otherwise bring an
/// emptied old folder back.
fn write_transcript(path: &std::path::Path, content: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        if !parent.is_dir() {
            return Err(format!(
                "Couldn't save the transcript: the folder {} no longer exists. The \
                 recording may have moved; reopen the meeting and try again.",
                parent.display()
            ));
        }
    }
    std::fs::write(path, content).map_err(|e| format!("Failed to write transcript: {}", e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_transcript_is_never_written_into_a_folder_that_is_gone() {
        let tmp = tempfile::tempdir().unwrap();
        let gone = tmp.path().join("old-root/Meeting");
        let err = write_transcript(&gone.join("transcript.json"), "{}").unwrap_err();
        assert!(err.contains("no longer exists"), "{err}");
        assert!(
            !tmp.path().join("old-root").exists(),
            "the folder was recreated"
        );

        write_transcript(&tmp.path().join("t.json"), "{}").unwrap();
        assert_eq!(
            std::fs::read_to_string(tmp.path().join("t.json")).unwrap(),
            "{}"
        );
    }
}
