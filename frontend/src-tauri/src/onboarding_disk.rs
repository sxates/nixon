//! Free-space check before the onboarding downloads (specs/0061 W1).
use serde::Serialize;
use std::path::Path;
use sysinfo::Disks;

pub const PARAKEET_MB: u64 = 670;

#[derive(Debug, Serialize)]
pub struct DiskCheck {
    pub free_bytes: u64,
    pub required_bytes: u64,
    pub ok: bool,
    pub models_dir: String,
}

pub fn evaluate(free: u64, required: u64) -> bool {
    free >= required.saturating_mul(2)
}

pub fn required_bytes(summary_model: Option<&str>) -> u64 {
    let summary_mb = summary_model
        .and_then(|m| {
            crate::summary::summary_engine::models::get_available_models()
                .into_iter()
                .find(|d| d.name == m)
                .map(|d| d.size_mb)
        })
        .unwrap_or(0);
    (PARAKEET_MB + summary_mb) * 1024 * 1024
}

/// Available bytes on the mount that contains `dir` (longest mount-point prefix wins).
pub fn free_space_for(dir: &Path) -> u64 {
    let disks = Disks::new_with_refreshed_list();
    let target = std::fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());
    disks
        .iter()
        .filter(|d| target.starts_with(d.mount_point()))
        .max_by_key(|d| d.mount_point().as_os_str().len())
        .map(|d| d.available_space())
        .unwrap_or(0)
}

#[tauri::command]
pub async fn get_models_disk_check(summary_model: Option<String>) -> Result<DiskCheck, String> {
    let models_dir = crate::app_paths::app_data_dir().join("models");
    let _ = std::fs::create_dir_all(&models_dir);
    let required = required_bytes(summary_model.as_deref());
    let mut free = free_space_for(&models_dir);
    #[cfg(debug_assertions)]
    if let Ok(v) = std::env::var("NIXON_FAKE_FREE_BYTES") {
        if crate::dev_fixtures::guard::allowed("NIXON_FAKE_FREE_BYTES") {
            if let Ok(n) = v.parse::<u64>() {
                free = n;
            }
        }
    }
    Ok(DiskCheck {
        free_bytes: free,
        required_bytes: required,
        ok: evaluate(free, required),
        models_dir: models_dir.to_string_lossy().into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ok_requires_double_headroom() {
        assert!(evaluate(2_000, 1_000));
        assert!(!evaluate(1_999, 1_000));
    }
    #[test]
    fn required_adds_parakeet_and_summary() {
        let r = required_bytes(Some("qwen3.5:2b"));
        assert_eq!(r, (670u64 + 1221) * 1024 * 1024);
        assert_eq!(required_bytes(None), 670 * 1024 * 1024);
        assert_eq!(required_bytes(Some("nope")), 670 * 1024 * 1024);
    }
    #[test]
    fn free_space_for_dir_reads_a_real_mount() {
        let f = free_space_for(&std::env::temp_dir());
        assert!(f > 0);
    }
}
