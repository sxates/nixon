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

// -- specs/0073 W1: known roots + the backend-owned previous folders ------------------

#[test]
fn an_older_preferences_file_has_no_previous_folders() {
    let prefs: RecordingPreferences =
        serde_json::from_str(r#"{"save_folder":"/tmp/x","auto_save":true}"#).unwrap();
    assert!(prefs.previous_save_folders.is_empty());
}

#[test]
fn known_roots_put_the_current_root_first_and_drop_duplicates() {
    let tmp = tempfile::tempdir().unwrap();
    let current = tmp.path().join("now");
    let old = tmp.path().join("before");
    std::fs::create_dir_all(&current).unwrap();
    std::fs::create_dir_all(&old).unwrap();
    // The same folder spelled through `..` is one root, not two.
    let old_again = current.join("..").join("before");

    let roots = known_roots_from(
        current.clone(),
        &[old.clone(), old_again, current.clone()],
        &[tmp.path().join("never-created")],
    );
    assert_eq!(
        roots,
        vec![
            current.canonicalize().unwrap(),
            old.canonicalize().unwrap(),
            tmp.path().join("never-created"),
        ]
    );
}

fn prefs_at(folder: &str, previous: &[&str]) -> RecordingPreferences {
    RecordingPreferences {
        save_folder: PathBuf::from(folder),
        previous_save_folders: previous.iter().map(PathBuf::from).collect(),
        ..RecordingPreferences::default()
    }
}

#[test]
fn a_settings_save_keeps_the_stored_previous_folders() {
    // The frontend never sends previous_save_folders; a save of some other field must not
    // wipe the list.
    let stored = prefs_at("/r/now", &["/r/old"]);
    let incoming = RecordingPreferences {
        auto_save: false,
        ..prefs_at("/r/now", &[])
    };
    let merged = carry_backend_fields(&stored, incoming);
    assert_eq!(merged.previous_save_folders, vec![PathBuf::from("/r/old")]);
    assert!(!merged.auto_save);
}

#[test]
fn changing_the_folder_remembers_the_outgoing_one_once() {
    let stored = prefs_at("/r/b", &["/r/a"]);
    let merged = carry_backend_fields(&stored, prefs_at("/r/c", &[]));
    assert_eq!(
        merged.previous_save_folders,
        vec![PathBuf::from("/r/a"), PathBuf::from("/r/b")]
    );

    // Switching back to an earlier folder makes it current, not previous.
    let merged = carry_backend_fields(&merged, prefs_at("/r/a", &[]));
    assert_eq!(merged.save_folder, PathBuf::from("/r/a"));
    assert_eq!(
        merged.previous_save_folders,
        vec![PathBuf::from("/r/b"), PathBuf::from("/r/c")]
    );
}

#[test]
fn a_settings_save_keeps_the_gather_answer() {
    // specs/0073: the frontend never sends recordings_gathered_once; a routine save must
    // not bring the one-time gather question back.
    let stored = RecordingPreferences {
        recordings_gathered_once: true,
        ..prefs_at("/r/now", &[])
    };
    let merged = carry_backend_fields(&stored, prefs_at("/r/now", &[]));
    assert!(merged.recordings_gathered_once);
}
