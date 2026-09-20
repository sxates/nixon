//! Prune stale bundled templates from the target directory (specs/0066 W1).
//!
//! `tauri.conf.json` lists `templates/*.json` as resources, which are copied to
//! `target/<profile>/templates/` and resolved from there by the dev build
//! (`set_bundled_templates_dir`). That copy only ever *adds*: a template deleted from
//! source stays behind in the target dir forever, and the app keeps serving it.
//!
//! That is not hypothetical. `psychatric_session.json` was deleted in specs/0061 W6 and
//! was still listed under Settings → Templates → Hidden on a dev build two specs later,
//! because `target/debug/templates/` still held a copy. Production bundles are built fresh
//! and were never affected, which is exactly why it went unnoticed for so long.
//!
//! So before the resources are staged, delete any `.json` in the target templates dir that
//! the source dir no longer has. Best-effort throughout: a build must not fail over
//! housekeeping.

use std::path::{Path, PathBuf};

/// `target/<profile>/`, derived from `OUT_DIR` (`target/<profile>/build/<pkg>-<hash>/out`).
fn profile_dir() -> Option<PathBuf> {
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").ok()?);
    Some(out_dir.ancestors().nth(3)?.to_path_buf())
}

/// The `.json` stems present in `dir`, or an empty set when it cannot be read.
fn json_names(dir: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|e| e.file_name().to_str().map(str::to_string))
        .filter(|name| name.ends_with(".json"))
        .collect()
}

/// Delete every `.json` in the staged templates dir that `source` no longer has.
pub fn prune_stale_bundled_templates() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default());
    let source = manifest.join("templates");
    println!("cargo:rerun-if-changed={}", source.display());

    let Some(staged) = profile_dir().map(|p| p.join("templates")) else {
        return;
    };
    if !staged.is_dir() || !source.is_dir() {
        return;
    }

    let current = json_names(&source);
    for name in json_names(&staged) {
        if current.contains(&name) {
            continue;
        }
        let path = staged.join(&name);
        match std::fs::remove_file(&path) {
            Ok(()) => println!(
                "cargo:warning=Removed stale bundled template {} (deleted from source)",
                name
            ),
            Err(e) => println!(
                "cargo:warning=Could not remove stale bundled template {:?}: {}",
                path, e
            ),
        }
    }
}
