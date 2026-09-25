//! The filesystem half of recovering a NULL-`folder_path` meeting's recording folder
//! (specs/0010 follow-up). Moved out of `pipeline.rs` for specs/0073 W1, which widened the
//! scan from the current recordings root to every recordings root Nixon knows about: a
//! meeting left in an earlier folder must still be diarizable. The name matching itself is
//! pure and lives in [`folder_match`](crate::diarization::folder_match).

use anyhow::{anyhow, Result};
use chrono::{DateTime, Utc};
use std::path::{Path, PathBuf};

use crate::audio::channel_writer::{mic_channel_path, system_channel_path, system_channel_wav};
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

/// Whether `folder` holds any per-channel audio: a system channel, or (specs/0078) a mic
/// channel alone, which a room recording can be. `.wav`, or `.opus` once compressed.
fn has_channel_audio(folder: &Path) -> bool {
    system_channel_path(folder).is_some() || mic_channel_path(folder).is_some()
}

/// Match the meeting's folder by `created_at`/`title` across `roots` and return it iff it
/// actually contains per-channel audio (see [`has_channel_audio`]). When the matched name
/// exists under several roots, the first root (in `roots` order) with one wins.
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
    if let Some(folder) = holders.iter().find(|f| has_channel_audio(f)) {
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

/// Resolve a meeting's recording folder for a diarization pass (moved out of
/// `pipeline.rs::resolve_system_wav` for specs/0078, which reads both channels).
///
/// Happy path: `meetings.folder_path` is set and holds per-channel audio → use it.
///
/// Fallback: `folder_path` is NULL (the frontend save can race the folder write) or points
/// somewhere without channel audio. We then scan every known recordings root and match a
/// folder by the meeting's `created_at`/title ([`locate_recording_folder`]). On a confident
/// match we use it and opportunistically backfill `folder_path`, so later reads (and "open
/// meeting folder") work without re-scanning. Errors are user-actionable.
pub(crate) async fn resolve_meeting_folder(
    pool: &sqlx::SqlitePool,
    meeting_id: &str,
) -> Result<PathBuf> {
    use crate::database::repositories::meeting::MeetingsRepository;
    use anyhow::Context;

    let meta = MeetingsRepository::get_meeting_metadata(pool, meeting_id)
        .await
        .with_context(|| format!("look up meeting {meeting_id}"))?
        .ok_or_else(|| anyhow!("meeting {meeting_id} not found"))?;

    if let Some(folder) = meta.folder_path.as_deref() {
        if has_channel_audio(Path::new(folder)) {
            return Ok(PathBuf::from(folder));
        }
        log::warn!(
            "meeting {meeting_id} folder_path is set ({folder}) but has no channel audio; \
             falling back to a recordings-root scan"
        );
    }

    let resolved = locate_recording_folder(&meta).with_context(|| {
        format!("locate recording folder for meeting {meeting_id} (folder_path was unusable)")
    })?;

    // Opportunistic backfill so we don't re-scan next time. Best-effort: log + continue.
    let folder_str = resolved.to_string_lossy().to_string();
    match MeetingsRepository::update_folder_path(pool, meeting_id, &folder_str).await {
        Ok(true) => log::info!("backfilled folder_path for meeting {meeting_id} -> {folder_str}"),
        Ok(false) => log::warn!("folder_path backfill for meeting {meeting_id} updated no rows"),
        Err(e) => log::warn!("folder_path backfill for meeting {meeting_id} failed: {e}"),
    }
    Ok(resolved)
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

    /// specs/0072: a meeting whose kept channels were compressed is still found.
    #[test]
    fn a_folder_with_a_compressed_system_channel_is_found() {
        let root = tempfile::tempdir().unwrap();
        let created = Utc.with_ymd_and_hms(2026, 7, 5, 17, 0, 47).unwrap();
        let folder = root.path().join(default_folder_name(created));
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("system.opus"), b"OggS").unwrap();
        let found = locate_recording_folder_in(created, "", &[root.path().to_path_buf()]);
        assert_eq!(found.unwrap(), folder);
    }

    /// specs/0078: a room recording's folder may hold only a mic channel.
    #[test]
    fn a_folder_with_only_a_mic_channel_is_found() {
        let root = tempfile::tempdir().unwrap();
        let created = Utc.with_ymd_and_hms(2026, 7, 5, 17, 0, 47).unwrap();
        let folder = root.path().join(default_folder_name(created));
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("mic.wav"), b"RIFF").unwrap();
        let found = locate_recording_folder_in(created, "", &[root.path().to_path_buf()]);
        assert_eq!(found.unwrap(), folder);
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
