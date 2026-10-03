//! Frozen, read-only reader for the pre-0079 summary templates (specs/0079 W5).
//!
//! Templates were replaced by saved prompts, but a user upgrading later from an older build
//! still has custom templates on disk and meetings that reference template ids. The one-time
//! conversion (`prompt_migration`) reads them through this module. It never writes or deletes
//! user files and keeps the old on-disk formats:
//! - custom templates: `<app-data>/templates/<id>.json` (id = file stem; dot-files skipped);
//!   a custom file shadows a built-in with the same id;
//! - hidden set: `<app-data>/templates/.hidden_templates.json`, a JSON array of ids.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde::Deserialize;

const HIDDEN_TEMPLATES_FILE: &str = ".hidden_templates.json";

/// The five templates that shipped with the app, frozen as embedded data.
pub(crate) const LEGACY_BUILTINS: [(&str, &str); 5] = [
    (
        "daily_standup",
        include_str!("legacy_templates/daily_standup.json"),
    ),
    (
        "standard_meeting",
        include_str!("legacy_templates/standard_meeting.json"),
    ),
    (
        "project_sync",
        include_str!("legacy_templates/project_sync.json"),
    ),
    (
        "retrospective",
        include_str!("legacy_templates/retrospective.json"),
    ),
    (
        "sales_marketing_client_call",
        include_str!("legacy_templates/sales_marketing_client_call.json"),
    ),
];

#[derive(Debug, Clone, Deserialize)]
pub struct TemplateSection {
    pub title: String,
    pub instruction: String,
    pub format: String,
    #[serde(default)]
    pub item_format: Option<String>,
    #[serde(default)]
    pub example_item_format: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Template {
    pub name: String,
    pub description: String,
    pub sections: Vec<TemplateSection>,
}

impl Template {
    fn validate(&self) -> Result<(), String> {
        if self.name.is_empty() || self.description.is_empty() || self.sections.is_empty() {
            return Err("Template needs a name, a description and at least one section".into());
        }
        for s in &self.sections {
            if s.title.is_empty() || s.instruction.is_empty() {
                return Err("Template section needs a title and an instruction".into());
            }
            if !matches!(s.format.as_str(), "paragraph" | "list" | "string") {
                return Err(format!("Section '{}' has invalid format", s.title));
            }
        }
        Ok(())
    }
}

/// `<app-data>/templates/` (identifier-derived, ADR-0004 — via `app_paths`).
pub fn custom_templates_dir() -> PathBuf {
    crate::app_paths::app_data_dir().join("templates")
}

fn builtin_json(id: &str) -> Option<&'static str> {
    LEGACY_BUILTINS
        .iter()
        .find(|(b, _)| *b == id)
        .map(|(_, json)| *json)
}

/// Built-in ids plus custom-file stems in `dir`, sorted and de-duplicated.
pub fn list_ids_in(dir: &Path) -> Vec<String> {
    let mut ids: BTreeSet<String> = LEGACY_BUILTINS
        .iter()
        .map(|(id, _)| id.to_string())
        .collect();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            if let Some(name) = entry.file_name().to_str() {
                if let Some(stem) = name.strip_suffix(".json") {
                    if !name.starts_with('.') {
                        ids.insert(stem.to_string());
                    }
                }
            }
        }
    }
    ids.into_iter().collect()
}

fn parse_and_validate(json: &str) -> Result<Template, String> {
    let t: Template =
        serde_json::from_str(json).map_err(|e| format!("Failed to parse template JSON: {e}"))?;
    t.validate()?;
    Ok(t)
}

/// Load a template: a valid custom file in `dir` wins over the built-in of the same id.
/// A custom file that is unparsable or invalid never hides a built-in: it falls back to the
/// embedded copy (only the id is logged, never the file's contents).
pub fn get_in(dir: &Path, id: &str) -> Result<Template, String> {
    if let Ok(json) = std::fs::read_to_string(dir.join(format!("{id}.json"))) {
        match parse_and_validate(&json) {
            Ok(t) => return Ok(t),
            Err(e) => {
                let Some(builtin) = builtin_json(id) else {
                    return Err(e);
                };
                log::warn!("Custom template '{id}' is invalid; using the built-in instead");
                return parse_and_validate(builtin);
            }
        }
    }
    let builtin = builtin_json(id).ok_or_else(|| format!("Template '{id}' not found"))?;
    parse_and_validate(builtin)
}

/// Hidden ids from `dir`'s sidecar; missing or corrupt means nothing is hidden.
pub fn hidden_ids_in(dir: &Path) -> BTreeSet<String> {
    std::fs::read_to_string(dir.join(HIDDEN_TEMPLATES_FILE))
        .ok()
        .and_then(|c| serde_json::from_str::<Vec<String>>(&c).ok())
        .map(|v| v.into_iter().collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn custom_file_shadows_builtin_and_hidden_file_is_read() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        let custom = r#"{"name":"Mine","description":"d","sections":[{"title":"T","instruction":"i","format":"list"}]}"#;
        std::fs::write(p.join("standard_meeting.json"), custom).unwrap();
        std::fs::write(p.join("extra.json"), custom).unwrap();
        std::fs::write(p.join(".hidden_templates.json"), r#"["extra"]"#).unwrap();
        let ids = list_ids_in(p);
        assert!(
            ids.contains(&"extra".to_string()) && ids.len() == 6,
            "{ids:?}"
        );
        assert_eq!(get_in(p, "standard_meeting").unwrap().name, "Mine");
        assert!(get_in(p, "daily_standup").is_ok());
        assert!(get_in(p, "nope").is_err());
        assert!(hidden_ids_in(p).contains("extra"));
        assert!(hidden_ids_in(&p.join("missing")).is_empty());
    }

    #[test]
    fn corrupt_override_of_a_builtin_falls_back_to_the_builtin() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        std::fs::write(p.join("standard_meeting.json"), "{ not json").unwrap();
        // Parses but fails validation (no sections).
        std::fs::write(
            p.join("daily_standup.json"),
            r#"{"name":"X","description":"d","sections":[]}"#,
        )
        .unwrap();
        let std = get_in(p, "standard_meeting").unwrap();
        assert!(!std.sections.is_empty());
        let standup = get_in(p, "daily_standup").unwrap();
        assert_ne!(standup.name, "X");
        // A corrupt file for a non-builtin id is still an error.
        std::fs::write(p.join("extra.json"), "{ not json").unwrap();
        assert!(get_in(p, "extra").is_err());
    }
}
