//! Agent-access preferences, persisted as `<data_dir>/agent_access.json`. The MCP server
//! reads this on every call, so flipping the switch in Settings takes effect immediately.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

const FILE_NAME: &str = "agent_access.json";

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentAccessSettings {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub registered_exe: Option<String>,
    #[serde(default)]
    pub registered_name: Option<String>,
}

/// Missing or unreadable/corrupt file means access is off.
pub fn load(data_dir: &Path) -> AgentAccessSettings {
    std::fs::read_to_string(data_dir.join(FILE_NAME))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// Writes a temp file then renames it, so a reader never sees a half-written file.
pub fn save(data_dir: &Path, s: &AgentAccessSettings) -> Result<()> {
    std::fs::create_dir_all(data_dir).context("Could not create Nixon's data folder")?;
    let tmp = data_dir.join(format!("{FILE_NAME}.tmp"));
    std::fs::write(&tmp, serde_json::to_string_pretty(s)?)
        .context("Could not save agent access settings")?;
    std::fs::rename(&tmp, data_dir.join(FILE_NAME))
        .context("Could not save agent access settings")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_is_off() {
        let d = tempfile::tempdir().unwrap();
        assert!(!load(d.path()).enabled);
    }

    #[test]
    fn corrupt_file_is_off() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join(FILE_NAME), "{not json").unwrap();
        assert_eq!(load(d.path()), AgentAccessSettings::default());
    }

    #[test]
    fn round_trips() {
        let d = tempfile::tempdir().unwrap();
        let s = AgentAccessSettings {
            enabled: true,
            registered_exe: Some("/Applications/Nixon.app/Contents/MacOS/nixon".into()),
            registered_name: Some("nixon".into()),
        };
        save(d.path(), &s).unwrap();
        assert_eq!(load(d.path()), s);
        assert!(!d.path().join("agent_access.json.tmp").exists());
    }
}
