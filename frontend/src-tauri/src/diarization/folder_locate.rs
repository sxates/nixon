//! The filesystem half of recovering a NULL-`folder_path` meeting's recording folder
//! (specs/0010 follow-up). Moved out of `pipeline.rs` for specs/0073 W1, which widened the
//! scan from the current recordings root to every recordings root Nixon knows about: a
//! meeting left in an earlier folder must still be diarizable. The name matching itself is
//! pure and lives in [`folder_match`](crate::diarization::folder_match).

use anyhow::{anyhow, Result};
use chrono::{DateTime, Utc};
use std::path::{Path, PathBuf};

use crate::audio::channel_writer::system_channel_wav;
use crate::diarization::folder_match::best_folder_match;

/// Bare directory names directly under `root` (files / non-UTF8 names skipped). A root
/// that can't be read contributes nothing.
fn folder_names(root: &Path) -> Vec<String> {
    let Ok(read_dir) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    read_dir
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .filter_map(|entry| entry.file_name().into_string().ok())
        .collect()
}

/// Match the meeting's folder by `created_at`/`title` across `roots` and return it iff it
/// actually contains a `system.wav`. When the matched name exists under several roots, the
/// first root (in `roots` order) whose copy has a `system.wav` wins.
pub fn locate_recording_folder_in(
    created_at: DateTime<Utc>,
    title: &str,
    roots: &[PathBuf],
) -> Result<PathBuf> {
    let mut candidates: Vec<String> = Vec::new();
    for root in roots {
        for name in folder_names(root) {
            if !candidates.contains(&name) {
                candidates.push(name);
            }
        }
    }

    let title = title.trim();
    let title_opt = (!title.is_empty()).then_some(title);
    let searched = roots
        .iter()
        .map(|r| r.display().to_string())
        .collect::<Vec<_>>()
        .join(", ");

    let name = best_folder_match(created_at, title_opt, &candidates).ok_or_else(|| {
        anyhow!(
            "no recording folder recorded for this meeting and none could be matched \
             under {searched}; it cannot be diarized"
        )
    })?;

    let holders: Vec<PathBuf> = roots
        .iter()
        .map(|r| r.join(&name))
        .filter(|f| f.is_dir())
        .collect();
    if let Some(folder) = holders.iter().find(|f| system_channel_wav(f).exists()) {
        return Ok(folder.clone());
    }
    let folder = holders
        .into_iter()
        .next()
        .unwrap_or_else(|| PathBuf::from(&name));
    Err(anyhow!(
        "matched recording folder {} has no per-channel audio (expected {}); \
         meetings recorded before per-channel capture cannot be diarized",
        folder.display(),
        system_channel_wav(&folder).display()
    ))
}

/// [`locate_recording_folder_in`] across every recordings root Nixon knows about.
pub fn locate_recording_folder(meta: &crate::database::models::MeetingModel) -> Result<PathBuf> {
    let roots = crate::audio::recording_preferences::known_recording_roots();
    locate_recording_folder_in(meta.created_at.0, &meta.title, &roots)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Local, TimeZone};

    /// A default-named folder for a meeting created at `created_at` (see `folder_match`).
    fn default_folder_name(created_at: DateTime<Utc>) -> String {
        let local = created_at.with_timezone(&Local);
        format!(
            "Meeting {}_{}",
            local.format("%Y-%m-%d_%H-%M-%S"),
            created_at.format("%Y-%m-%d_%H-%M")
        )
    }

    #[test]
    fn a_folder_in_an_earlier_root_is_found() {
        let current = tempfile::tempdir().unwrap();
        let earlier = tempfile::tempdir().unwrap();
        let created = Utc.with_ymd_and_hms(2026, 7, 5, 17, 0, 47).unwrap();
        let folder = earlier.path().join(default_folder_name(created));
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(system_channel_wav(&folder), b"RIFF").unwrap();

        let roots = [current.path().to_path_buf(), earlier.path().to_path_buf()];
        let found = locate_recording_folder_in(created, "", &roots).unwrap();
        assert_eq!(found, folder);
    }

    #[test]
    fn a_match_without_channel_audio_is_an_actionable_error() {
        let root = tempfile::tempdir().unwrap();
        let created = Utc.with_ymd_and_hms(2026, 7, 5, 17, 0, 47).unwrap();
        std::fs::create_dir_all(root.path().join(default_folder_name(created))).unwrap();
        let err = locate_recording_folder_in(created, "", &[root.path().to_path_buf()])
            .unwrap_err()
            .to_string();
        assert!(err.contains("no per-channel audio"), "{err}");
    }

    #[test]
    fn no_match_names_every_root_searched() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        let created = Utc.with_ymd_and_hms(2026, 7, 5, 17, 0, 47).unwrap();
        let err = locate_recording_folder_in(
            created,
            "",
            &[a.path().to_path_buf(), b.path().to_path_buf()],
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains(&a.path().display().to_string()), "{err}");
        assert!(err.contains(&b.path().display().to_string()), "{err}");
    }
}
