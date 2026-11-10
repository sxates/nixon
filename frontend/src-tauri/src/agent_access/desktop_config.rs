//! Adding `nixon --mcp` to the Claude desktop app's chat side (specs/0087 Amendment A).
//!
//! The chat side reads its own MCP list from `claude_desktop_config.json`, which
//! `claude mcp add` never touches. This file belongs to another app, so every edit keeps
//! all other keys and servers, writes a temp file then renames it (keeping the file's
//! permissions), skips the write when nothing changes, and refuses to touch a file it
//! cannot parse.

use super::register::Profile;
use serde_json::{json, Map, Value};
use std::path::{Path, PathBuf};

const FILE_NAME: &str = "claude_desktop_config.json";

/// `~/Library/Application Support/Claude/claude_desktop_config.json`.
pub fn config_path(home: &Path) -> PathBuf {
    home.join("Library/Application Support/Claude")
        .join(FILE_NAME)
}

/// Claude desktop counts as installed when its support folder exists. Nixon never creates it.
pub fn is_installed(config: &Path) -> bool {
    config.parent().is_some_and(Path::is_dir)
}

/// The entry Claude desktop launches: the same args the Claude Code entry passes after `--`.
pub fn server_entry(p: &Profile, exe: &Path, data_dir: &Path) -> Value {
    json!({
        "command": exe.to_string_lossy(),
        "args": [
            "--mcp",
            "--data-dir",
            data_dir.to_string_lossy(),
            "--link-scheme",
            p.link_scheme,
        ],
    })
}

fn read_root(config: &Path) -> Result<Map<String, Value>, String> {
    let text = match std::fs::read_to_string(config) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Map::new()),
        Err(e) => return Err(format!("Could not read Claude's settings file: {e}")),
    };
    if text.trim().is_empty() {
        return Ok(Map::new());
    }
    match serde_json::from_str::<Value>(&text) {
        Ok(Value::Object(m)) => Ok(m),
        _ => Err(
            "Claude's settings file isn't in a format Nixon understands, so Nixon left it alone."
                .into(),
        ),
    }
}

fn write_root(config: &Path, root: &Map<String, Value>) -> Result<(), String> {
    let fail = |e: std::io::Error| format!("Could not update Claude's settings file: {e}");
    let mut text = serde_json::to_string_pretty(root).map_err(|e| e.to_string())?;
    text.push('\n');
    let tmp = config.with_file_name(format!("{FILE_NAME}.nixon-tmp"));
    std::fs::write(&tmp, text).map_err(fail)?;
    if let Ok(meta) = std::fs::metadata(config) {
        let _ = std::fs::set_permissions(&tmp, meta.permissions());
    }
    std::fs::rename(&tmp, config).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        fail(e)
    })
}

fn servers_mut(root: &mut Map<String, Value>) -> Result<&mut Map<String, Value>, String> {
    root.entry("mcpServers")
        .or_insert_with(|| Value::Object(Map::new()))
        .as_object_mut()
        .ok_or_else(|| {
            "Claude's settings file lists its servers in an unexpected way, so Nixon left it alone."
                .to_string()
        })
}

/// Adds or updates Nixon's entry. `Ok(false)` when Claude desktop isn't installed.
pub fn add(config: &Path, p: &Profile, exe: &Path, data_dir: &Path) -> Result<bool, String> {
    if !is_installed(config) {
        return Ok(false);
    }
    let mut root = read_root(config)?;
    let entry = server_entry(p, exe, data_dir);
    let servers = servers_mut(&mut root)?;
    if servers.get(p.server_name) == Some(&entry) {
        return Ok(true);
    }
    servers.insert(p.server_name.to_string(), entry);
    write_root(config, &root)?;
    Ok(true)
}

/// Removes Nixon's entry, leaving everything else. A missing file or entry is fine.
pub fn remove(config: &Path, p: &Profile) -> Result<(), String> {
    if !config.exists() {
        return Ok(());
    }
    let mut root = read_root(config)?;
    let Some(Value::Object(servers)) = root.get_mut("mcpServers") else {
        return Ok(());
    };
    if servers.remove(p.server_name).is_none() {
        return Ok(());
    }
    write_root(config, &root)
}

/// Whether Nixon's entry is present (with any command).
pub fn is_registered(config: &Path, p: &Profile) -> bool {
    read_root(config)
        .ok()
        .and_then(|r| r.get("mcpServers")?.get(p.server_name).cloned())
        .is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_access::register::profile_for;

    fn setup() -> (tempfile::TempDir, PathBuf) {
        let home = tempfile::tempdir().unwrap();
        let cfg = config_path(home.path());
        std::fs::create_dir_all(cfg.parent().unwrap()).unwrap();
        (home, cfg)
    }

    fn read(cfg: &Path) -> Value {
        serde_json::from_str(&std::fs::read_to_string(cfg).unwrap()).unwrap()
    }

    #[test]
    fn not_installed_writes_nothing() {
        let home = tempfile::tempdir().unwrap();
        let cfg = config_path(home.path());
        let p = profile_for("ai.vinyl.app");
        assert_eq!(
            add(&cfg, &p, Path::new("/a/nixon"), Path::new("/d")),
            Ok(false)
        );
        assert!(!cfg.parent().unwrap().exists());
    }

    #[test]
    fn add_to_missing_file_creates_entry_with_server_args() {
        let (_h, cfg) = setup();
        let p = profile_for("ai.vinyl.app.debug");
        assert_eq!(
            add(&cfg, &p, Path::new("/a/nixon"), Path::new("/d d")),
            Ok(true)
        );
        let v = read(&cfg);
        assert_eq!(v["mcpServers"]["nixon-dev"]["command"], "/a/nixon");
        let args: Vec<String> =
            serde_json::from_value(v["mcpServers"]["nixon-dev"]["args"].clone()).unwrap();
        assert_eq!(
            args,
            ["--mcp", "--data-dir", "/d d", "--link-scheme", "nixon-dev"]
        );
        // The server must accept exactly what we register.
        assert!(crate::agent_access::entry::parse_mcp_args(&args)
            .unwrap()
            .is_ok());
    }

    #[test]
    fn add_and_remove_keep_other_keys_and_servers() {
        let (_h, cfg) = setup();
        let original = json!({
            "preferences": {"sidebarMode": "chat", "quickEntry": true},
            "mcpServers": {"other": {"command": "/x", "args": ["1"], "env": {"K": "V"}}},
            "unknownFutureKey": [1, 2, 3]
        });
        std::fs::write(&cfg, serde_json::to_string(&original).unwrap()).unwrap();
        let p = profile_for("ai.vinyl.app");
        add(&cfg, &p, Path::new("/a/nixon"), Path::new("/d")).unwrap();
        let v = read(&cfg);
        assert_eq!(v["preferences"], original["preferences"]);
        assert_eq!(v["unknownFutureKey"], original["unknownFutureKey"]);
        assert_eq!(v["mcpServers"]["other"], original["mcpServers"]["other"]);
        assert!(is_registered(&cfg, &p));
        remove(&cfg, &p).unwrap();
        assert_eq!(read(&cfg), original);
        assert!(!is_registered(&cfg, &p));
    }

    #[test]
    fn add_updates_a_moved_exe_and_skips_identical_writes() {
        let (_h, cfg) = setup();
        let p = profile_for("ai.vinyl.app");
        add(&cfg, &p, Path::new("/old/nixon"), Path::new("/d")).unwrap();
        add(&cfg, &p, Path::new("/new/nixon"), Path::new("/d")).unwrap();
        assert_eq!(read(&cfg)["mcpServers"]["nixon"]["command"], "/new/nixon");
        // Identical entry: no rewrite (a sentinel formatting survives).
        let compact = serde_json::to_string(&read(&cfg)).unwrap();
        std::fs::write(&cfg, &compact).unwrap();
        add(&cfg, &p, Path::new("/new/nixon"), Path::new("/d")).unwrap();
        assert_eq!(std::fs::read_to_string(&cfg).unwrap(), compact);
    }

    #[test]
    fn invalid_json_is_left_alone() {
        let (_h, cfg) = setup();
        for bad in ["{not json", "[1,2]", r#"{"mcpServers": [1]}"#] {
            std::fs::write(&cfg, bad).unwrap();
            let p = profile_for("ai.vinyl.app");
            assert!(
                add(&cfg, &p, Path::new("/a/nixon"), Path::new("/d")).is_err(),
                "{bad}"
            );
            assert_eq!(std::fs::read_to_string(&cfg).unwrap(), bad);
        }
        std::fs::write(&cfg, "{not json").unwrap();
        assert!(remove(&cfg, &profile_for("ai.vinyl.app")).is_err());
        assert_eq!(std::fs::read_to_string(&cfg).unwrap(), "{not json");
    }

    #[test]
    fn remove_without_file_or_entry_is_ok_and_writes_nothing() {
        let (_h, cfg) = setup();
        let p = profile_for("ai.vinyl.app");
        assert_eq!(remove(&cfg, &p), Ok(()));
        assert!(!cfg.exists());
        std::fs::write(&cfg, r#"{"mcpServers":{}}"#).unwrap();
        assert_eq!(remove(&cfg, &p), Ok(()));
        assert_eq!(
            std::fs::read_to_string(&cfg).unwrap(),
            r#"{"mcpServers":{}}"#
        );
    }

    #[cfg(unix)]
    #[test]
    fn keeps_file_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let (_h, cfg) = setup();
        std::fs::write(&cfg, "{}").unwrap();
        std::fs::set_permissions(&cfg, std::fs::Permissions::from_mode(0o600)).unwrap();
        add(
            &cfg,
            &profile_for("ai.vinyl.app"),
            Path::new("/a/nixon"),
            Path::new("/d"),
        )
        .unwrap();
        let mode = std::fs::metadata(&cfg).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        assert!(!cfg
            .with_file_name(format!("{FILE_NAME}.nixon-tmp"))
            .exists());
    }
}
