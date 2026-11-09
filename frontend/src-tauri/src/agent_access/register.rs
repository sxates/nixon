//! Registering `nixon --mcp` with Claude Code (specs/0087). Everything here is pure or takes
//! an injected runner so tests never call the real `claude`.

use super::settings::{self, AgentAccessSettings};
use serde::Serialize;
use std::ffi::{OsStr, OsString};
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Profile {
    pub server_name: &'static str,
    pub link_scheme: &'static str,
}

pub fn profile_for(bundle_id: &str) -> Profile {
    if bundle_id.ends_with(".debug") {
        Profile {
            server_name: "nixon-dev",
            link_scheme: "nixon-dev",
        }
    } else {
        Profile {
            server_name: "nixon",
            link_scheme: "nixon",
        }
    }
}

/// Arguments for `claude`, exactly what `entry::parse_mcp_args` accepts after the `--`.
pub fn add_argv(p: &Profile, exe: &Path, data_dir: &Path) -> Vec<String> {
    [
        "mcp",
        "add",
        "--scope",
        "user",
        p.server_name,
        "--",
        &exe.to_string_lossy(),
        "--mcp",
        "--data-dir",
        &data_dir.to_string_lossy(),
        "--link-scheme",
        p.link_scheme,
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

pub fn remove_argv(p: &Profile) -> Vec<String> {
    ["mcp", "remove", "--scope", "user", p.server_name]
        .iter()
        .map(|s| s.to_string())
        .collect()
}

fn shell_quote(s: &str) -> String {
    let safe = !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || "/._-:=@%+,".contains(c));
    if safe {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', "'\\''"))
    }
}

/// The `claude …` line the user can paste into a terminal when Nixon cannot run it.
pub fn manual_command(p: &Profile, exe: &Path, data_dir: &Path) -> String {
    let mut parts = vec!["claude".to_string()];
    parts.extend(add_argv(p, exe, data_dir).iter().map(|a| shell_quote(a)));
    parts.join(" ")
}

pub fn needs_reregister(s: &AgentAccessSettings, current_exe: &Path, name: &str) -> bool {
    s.enabled
        && (s.registered_exe.as_deref() != Some(&*current_exe.to_string_lossy())
            || s.registered_name.as_deref() != Some(name))
}

/// PATH for the `claude` child: its own directory first (resolves `node` for npm, nvm and
/// Volta installs), then the inherited PATH, then the 0085 base PATH. A Finder-launched app
/// inherits only `/usr/bin:/bin:...`, which cannot find `node`.
pub fn child_path(claude: &Path, inherited: Option<&OsStr>) -> OsString {
    let mut path = OsString::new();
    if let Some(dir) = claude.parent().filter(|d| !d.as_os_str().is_empty()) {
        path.push(dir.as_os_str());
        path.push(":");
    }
    if let Some(p) = inherited.filter(|p| !p.is_empty()) {
        path.push(p);
        path.push(":");
    }
    path.push(crate::cli_llm::runner::BASE_PATH);
    path
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentAccessStatus {
    pub enabled: bool,
    pub registered: bool,
    pub server_name: String,
    pub manual_command: String,
    pub last_error: Option<String>,
}

/// Runs `claude` with the given args; `Err` carries a user-readable reason.
pub type Runner<'a> = &'a dyn Fn(&[String]) -> Result<(), String>;

fn status_of(
    s: &AgentAccessSettings,
    p: &Profile,
    exe: &Path,
    data_dir: &Path,
    err: Option<String>,
) -> AgentAccessStatus {
    AgentAccessStatus {
        enabled: s.enabled,
        registered: s.enabled && s.registered_exe.is_some() && err.is_none(),
        server_name: p.server_name.to_string(),
        manual_command: manual_command(p, exe, data_dir),
        last_error: err,
    }
}

/// Add is always preceded by a remove whose failure is ignored (idempotent).
fn register(p: &Profile, exe: &Path, data_dir: &Path, run: Runner) -> Result<(), String> {
    let _ = run(&remove_argv(p));
    run(&add_argv(p, exe, data_dir))
}

/// Writes the setting first, then registers or unregisters. A `claude` failure leaves
/// `enabled` as requested and reports `last_error` with `registered: false`.
pub fn set_enabled(
    enabled: bool,
    p: &Profile,
    exe: &Path,
    data_dir: &Path,
    run: Runner,
) -> anyhow::Result<AgentAccessStatus> {
    let mut s = settings::load(data_dir);
    s.enabled = enabled;
    settings::save(data_dir, &s)?;
    let outcome = if enabled {
        register(p, exe, data_dir, run)
    } else {
        run(&remove_argv(p))
    };
    let err = match outcome {
        Ok(()) => {
            if enabled {
                s.registered_exe = Some(exe.to_string_lossy().into_owned());
                s.registered_name = Some(p.server_name.to_string());
            } else {
                s.registered_exe = None;
                s.registered_name = None;
            }
            settings::save(data_dir, &s)?;
            None
        }
        Err(e) => Some(e),
    };
    Ok(status_of(&s, p, exe, data_dir, err))
}

/// Startup refresh: re-adds the entry when the app moved or was renamed. Best effort.
pub fn refresh_if_needed(p: &Profile, exe: &Path, data_dir: &Path, run: Runner) -> Option<String> {
    let mut s = settings::load(data_dir);
    if !needs_reregister(&s, exe, p.server_name) {
        return None;
    }
    match register(p, exe, data_dir, run) {
        Ok(()) => {
            s.registered_exe = Some(exe.to_string_lossy().into_owned());
            s.registered_name = Some(p.server_name.to_string());
            settings::save(data_dir, &s).err().map(|e| e.to_string())
        }
        Err(e) => Some(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::path::PathBuf;

    fn dev() -> Profile {
        profile_for("ai.vinyl.app.debug")
    }

    #[test]
    fn profile_for_debug_and_prod() {
        assert_eq!(
            dev(),
            Profile {
                server_name: "nixon-dev",
                link_scheme: "nixon-dev"
            }
        );
        assert_eq!(
            profile_for("ai.vinyl.app"),
            Profile {
                server_name: "nixon",
                link_scheme: "nixon"
            }
        );
    }

    #[test]
    fn add_argv_exact() {
        let v = add_argv(&dev(), Path::new("/a/nixon"), Path::new("/d"));
        assert_eq!(
            v,
            [
                "mcp",
                "add",
                "--scope",
                "user",
                "nixon-dev",
                "--",
                "/a/nixon",
                "--mcp",
                "--data-dir",
                "/d",
                "--link-scheme",
                "nixon-dev"
            ]
        );
        // The server must accept exactly what we register.
        let rest: Vec<String> = v[6..].to_vec();
        assert!(super::super::entry::parse_mcp_args(&rest).unwrap().is_ok());
    }

    #[test]
    fn child_path_puts_claude_dir_first_and_appends_base_path() {
        let base = crate::cli_llm::runner::BASE_PATH;
        let p = child_path(
            Path::new("/u/.npm/bin/claude"),
            Some(OsStr::new("/usr/bin:/bin")),
        );
        assert_eq!(
            p,
            OsString::from(format!("/u/.npm/bin:/usr/bin:/bin:{base}"))
        );
        let p = child_path(Path::new("/u/.npm/bin/claude"), None);
        assert_eq!(p, OsString::from(format!("/u/.npm/bin:{base}")));
    }

    #[test]
    fn remove_argv_exact() {
        assert_eq!(
            remove_argv(&profile_for("ai.vinyl.app")),
            ["mcp", "remove", "--scope", "user", "nixon"]
        );
    }

    #[test]
    fn manual_command_quotes_paths_with_spaces() {
        let c = manual_command(
            &profile_for("ai.vinyl.app"),
            Path::new("/Applications/My Apps/Nixon.app/Contents/MacOS/nixon"),
            Path::new("/Users/x/Library/Application Support/ai.vinyl.app"),
        );
        assert_eq!(
            c,
            "claude mcp add --scope user nixon -- '/Applications/My Apps/Nixon.app/Contents/MacOS/nixon' --mcp --data-dir '/Users/x/Library/Application Support/ai.vinyl.app' --link-scheme nixon"
        );
    }

    fn settings_for(enabled: bool, exe: &str) -> AgentAccessSettings {
        AgentAccessSettings {
            enabled,
            registered_exe: Some(exe.into()),
            registered_name: Some("nixon".into()),
        }
    }

    #[test]
    fn needs_reregister_when_exe_moved() {
        let s = settings_for(true, "/old/nixon");
        assert!(needs_reregister(&s, Path::new("/new/nixon"), "nixon"));
        assert!(!needs_reregister(&s, Path::new("/old/nixon"), "nixon"));
        assert!(needs_reregister(&s, Path::new("/old/nixon"), "nixon-dev"));
        assert!(needs_reregister(
            &AgentAccessSettings {
                enabled: true,
                ..Default::default()
            },
            Path::new("/old/nixon"),
            "nixon"
        ));
    }

    #[test]
    fn no_reregister_when_disabled() {
        let s = settings_for(false, "/old/nixon");
        assert!(!needs_reregister(&s, Path::new("/new/nixon"), "nixon"));
    }

    #[test]
    fn set_enabled_records_failure_with_fake_runner() {
        let d = tempfile::tempdir().unwrap();
        let calls = RefCell::new(Vec::<Vec<String>>::new());
        let fail = |a: &[String]| {
            calls.borrow_mut().push(a.to_vec());
            Err("claude exploded".to_string())
        };
        let st = set_enabled(true, &dev(), Path::new("/a/nixon"), d.path(), &fail).unwrap();
        assert!(st.enabled);
        assert!(!st.registered);
        assert_eq!(st.last_error.as_deref(), Some("claude exploded"));
        assert!(settings::load(d.path()).enabled);
        // remove (ignored) then add
        assert_eq!(calls.borrow().len(), 2);
        assert_eq!(calls.borrow()[0][1], "remove");
    }

    #[test]
    fn set_enabled_success_and_disable() {
        let d = tempfile::tempdir().unwrap();
        let ok = |_: &[String]| Ok(());
        let st = set_enabled(true, &dev(), Path::new("/a/nixon"), d.path(), &ok).unwrap();
        assert!(st.registered && st.last_error.is_none());
        assert_eq!(
            settings::load(d.path()).registered_exe.as_deref(),
            Some("/a/nixon")
        );
        let st = set_enabled(false, &dev(), Path::new("/a/nixon"), d.path(), &ok).unwrap();
        assert!(!st.enabled && !st.registered);
        assert_eq!(settings::load(d.path()).registered_exe, None);
    }

    #[test]
    fn refresh_readds_after_move() {
        let d = tempfile::tempdir().unwrap();
        settings::save(d.path(), &settings_for(true, "/old/nixon")).unwrap();
        let ok = |_: &[String]| Ok(());
        let p = profile_for("ai.vinyl.app");
        assert_eq!(
            refresh_if_needed(&p, &PathBuf::from("/new/nixon"), d.path(), &ok),
            None
        );
        assert_eq!(
            settings::load(d.path()).registered_exe.as_deref(),
            Some("/new/nixon")
        );
    }
}
