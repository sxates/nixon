//! specs/0092 T1 — manifest validation, build schema version, and which files a backup
//! selects. Temp dirs only.
//!
//! Run with: cargo test --features metal --test backup_manifest

use std::collections::BTreeSet;
use std::path::Path;

use app_lib::backup::files::{folder_entries, settings_entries, SETTINGS_ALLOWLIST};
use app_lib::backup::manifest::{build_schema_version, validate, Counts, Manifest, FORMAT};
use app_lib::backup::BackupError;

fn manifest(schema_version: i64) -> Manifest {
    Manifest {
        format: FORMAT,
        app: "Nixon".into(),
        app_version: "1.2.3".into(),
        schema_version,
        created_at: "2026-11-24T09:30:00Z".into(),
        includes_audio: false,
        counts: Counts::default(),
        recordings: vec![],
    }
}

fn touch(path: &Path) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, b"x").unwrap();
}

#[test]
fn validate_rejects_newer_schema() {
    let build = build_schema_version();
    let err = validate(&manifest(build + 1), build).unwrap_err();
    assert!(matches!(
        err,
        BackupError::NewerSchema { backup, build: b, ref app_version }
            if backup == build + 1 && b == build && app_version == "1.2.3"
    ));
}

#[test]
fn validate_accepts_older_and_equal_schema() {
    let build = build_schema_version();
    assert!(validate(&manifest(build), build).is_ok());
    assert!(validate(&manifest(build - 1), build).is_ok());
}

#[test]
fn validate_rejects_unknown_format() {
    let mut m = manifest(build_schema_version());
    m.format = 2;
    assert!(matches!(
        validate(&m, build_schema_version()),
        Err(BackupError::UnsupportedFormat(2))
    ));
}

#[test]
fn build_schema_version_is_latest_migration() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
    let max = std::fs::read_dir(dir)
        .unwrap()
        .flatten()
        .filter_map(|e| {
            e.file_name()
                .to_string_lossy()
                .split('_')
                .next()
                .and_then(|p| p.parse::<i64>().ok())
        })
        .max()
        .unwrap();
    assert!(max >= 20261117000000);
    assert_eq!(build_schema_version(), max);
}

#[test]
fn error_serializes_kind_and_message() {
    let v = serde_json::to_value(BackupError::UnsupportedFormat(2)).unwrap();
    assert_eq!(v["kind"], "unsupportedFormat");
    assert!(v["message"].as_str().unwrap().contains('2'));
    let v = serde_json::to_value(BackupError::NotABackup).unwrap();
    assert_eq!(v["kind"], "notABackup");
}

#[test]
fn folder_entries_skips_media_and_checkpoints() {
    let dir = tempfile::tempdir().unwrap();
    for f in [
        "audio.opus",
        "mic.wav",
        "metadata.json",
        "transcripts.json",
        ".checkpoints/x.wav",
    ] {
        touch(&dir.path().join(f));
    }
    let names = |audio| -> BTreeSet<String> {
        folder_entries(dir.path(), audio)
            .iter()
            .map(|p| {
                p.strip_prefix(dir.path())
                    .unwrap()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect()
    };
    let without: BTreeSet<String> = ["metadata.json", "transcripts.json"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    assert_eq!(names(false), without);
    let with: BTreeSet<String> = ["metadata.json", "transcripts.json", "audio.opus", "mic.wav"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    assert_eq!(names(true), with);
}

#[test]
fn settings_entries_only_allowlist() {
    let dir = tempfile::tempdir().unwrap();
    for f in SETTINGS_ALLOWLIST {
        touch(&dir.path().join(f));
    }
    touch(&dir.path().join("templates/sub/a.json"));
    for f in [
        "analytics.json",
        "agent_access.json",
        "onboarding-status.json",
        "meeting_minutes.sqlite.backup-1",
    ] {
        touch(&dir.path().join(f));
    }
    let got: BTreeSet<String> = settings_entries(dir.path())
        .into_iter()
        .map(|(_, z)| z)
        .collect();
    let mut want: BTreeSet<String> = SETTINGS_ALLOWLIST.iter().map(|s| s.to_string()).collect();
    want.insert("templates/sub/a.json".into());
    assert_eq!(got, want);
    assert!(!got.contains("analytics.json"));
}
