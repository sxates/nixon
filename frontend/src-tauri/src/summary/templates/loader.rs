use super::defaults;
use super::types::Template;
use once_cell::sync::Lazy;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::RwLock;
use tracing::{debug, info, warn};

/// Sidecar file in the custom templates dir recording which SHIPPED template ids
/// the user removed from their list (specs/0020 follow-up: built-ins are
/// hideable, not deletable). Dot-prefixed so the custom-dir scan skips it.
/// Hidden ≠ deleted: `get_template` still resolves hidden ids, so old meetings
/// that persisted one keep summarizing with it.
const HIDDEN_TEMPLATES_FILE: &str = ".hidden_templates.json";

// Global storage for the bundled templates directory path
static BUNDLED_TEMPLATES_DIR: Lazy<RwLock<Option<PathBuf>>> = Lazy::new(|| RwLock::new(None));

/// Set the bundled templates directory path (called once at app startup)
pub fn set_bundled_templates_dir(path: PathBuf) {
    info!("Bundled templates directory set to: {:?}", path);
    if let Ok(mut dir) = BUNDLED_TEMPLATES_DIR.write() {
        *dir = Some(path);
    }
}

/// Get the user's custom templates directory path
///
/// Returns the identifier-derived application data directory for custom
/// templates (`<app-data-dir>/templates/`), so dev and prod stay isolated
/// (ADR-0004). See [`crate::app_paths`].
pub fn get_custom_templates_dir() -> Option<PathBuf> {
    Some(crate::app_paths::app_data_dir().join("templates"))
}

/// True when a file for this template id exists in the user's custom dir.
fn custom_template_file_exists(template_id: &str) -> bool {
    get_custom_templates_dir()
        .map(|dir| dir.join(format!("{}.json", template_id)).is_file())
        .unwrap_or(false)
}

/// True when a file for this template id exists in the bundled resources dir.
fn bundled_template_file_exists(template_id: &str) -> bool {
    BUNDLED_TEMPLATES_DIR
        .read()
        .ok()
        .and_then(|dir| dir.clone())
        .map(|dir| dir.join(format!("{}.json", template_id)).is_file())
        .unwrap_or(false)
}

/// True when the id ships with the app (embedded built-in or bundled resource),
/// i.e. it exists independently of any custom-dir file. Used by the delete
/// command: such ids can only be *overridden*, never deleted.
pub fn is_builtin_or_bundled(template_id: &str) -> bool {
    defaults::get_builtin_template(template_id).is_some()
        || bundled_template_file_exists(template_id)
}

/// Read the hidden-template id set from `dir`'s sidecar file. Missing or
/// unreadable/corrupt file = nothing hidden (hiding is cosmetic, never fail).
pub(crate) fn read_hidden_ids_from(dir: &Path) -> BTreeSet<String> {
    let path = dir.join(HIDDEN_TEMPLATES_FILE);
    match std::fs::read_to_string(&path) {
        Ok(content) => serde_json::from_str::<Vec<String>>(&content)
            .map(|ids| ids.into_iter().collect())
            .unwrap_or_else(|e| {
                warn!("Ignoring corrupt {:?}: {}", path, e);
                BTreeSet::new()
            }),
        Err(_) => BTreeSet::new(),
    }
}

/// Add or remove `id` in `dir`'s hidden set (read-modify-write; creates the
/// dir/file on first hide). No-op writes (already hidden/visible) still succeed.
pub(crate) fn set_hidden_in_dir(dir: &Path, id: &str, hidden: bool) -> Result<(), String> {
    let mut ids = read_hidden_ids_from(dir);
    let changed = if hidden {
        ids.insert(id.to_string())
    } else {
        ids.remove(id)
    };
    if !changed {
        return Ok(());
    }
    std::fs::create_dir_all(dir)
        .map_err(|e| format!("Failed to create the templates directory {:?}: {}", dir, e))?;
    let path = dir.join(HIDDEN_TEMPLATES_FILE);
    let json = serde_json::to_string_pretty(&ids.iter().collect::<Vec<_>>())
        .map_err(|e| format!("Failed to serialize hidden-template list: {}", e))?;
    std::fs::write(&path, json)
        .map_err(|e| format!("Failed to write hidden-template list to {:?}: {}", path, e))
}

/// The user's hidden-template ids (empty when none / no custom dir).
pub fn hidden_template_ids() -> BTreeSet<String> {
    get_custom_templates_dir()
        .map(|dir| read_hidden_ids_from(&dir))
        .unwrap_or_default()
}

/// Hide or restore a template id in the user's list.
pub fn set_template_hidden(id: &str, hidden: bool) -> Result<(), String> {
    let dir = get_custom_templates_dir()
        .ok_or_else(|| "Custom templates directory is unavailable".to_string())?;
    set_hidden_in_dir(&dir, id, hidden)
}

/// Pure classification of where a template id resolves from (specs/0020 task 5):
/// - `"custom"`  — a custom-dir file whose id does not shadow anything shipped
/// - `"override"` — a custom-dir file shadowing a built-in/bundled id
/// - `"builtin"` — everything else (embedded built-ins and bundled-dir templates)
pub(crate) fn classify_source(has_custom_file: bool, shadows_shipped: bool) -> &'static str {
    match (has_custom_file, shadows_shipped) {
        (true, true) => "override",
        (true, false) => "custom",
        (false, _) => "builtin",
    }
}

/// Template ids the app owns and a user template may never shadow.
///
/// specs/0053 W3: `auto` is content-derived structure, resolved at runtime — it
/// is never a file on disk, so a custom template saved under that id would
/// silently take precedence over it in `get_template`'s lookup order.
pub fn is_reserved_template_id(template_id: &str) -> bool {
    template_id == crate::summary::outline::AUTO_TEMPLATE_ID
}

/// Where the given template id currently resolves from ("custom" | "override" |
/// "builtin"), following the same custom-first precedence as [`get_template`].
pub fn template_source(template_id: &str) -> &'static str {
    classify_source(
        custom_template_file_exists(template_id),
        is_builtin_or_bundled(template_id),
    )
}

/// Load a template from the bundled resources directory
///
/// # Arguments
/// * `template_id` - Template identifier (without .json extension)
///
/// # Returns
/// The template JSON content if found, None otherwise
fn load_bundled_template(template_id: &str) -> Option<String> {
    let bundled_dir = BUNDLED_TEMPLATES_DIR.read().ok()?.clone()?;
    let template_path = bundled_dir.join(format!("{}.json", template_id));

    debug!("Checking for bundled template at: {:?}", template_path);

    match std::fs::read_to_string(&template_path) {
        Ok(content) => {
            info!(
                "Loaded bundled template '{}' from {:?}",
                template_id, template_path
            );
            Some(content)
        }
        Err(e) => {
            debug!("No bundled template '{}' found: {}", template_id, e);
            None
        }
    }
}

/// Load a template from the user's custom templates directory
///
/// # Arguments
/// * `template_id` - Template identifier (without .json extension)
///
/// # Returns
/// The template JSON content if found, None otherwise
fn load_custom_template(template_id: &str) -> Option<String> {
    let custom_dir = get_custom_templates_dir()?;
    let template_path = custom_dir.join(format!("{}.json", template_id));

    debug!("Checking for custom template at: {:?}", template_path);

    match std::fs::read_to_string(&template_path) {
        Ok(content) => {
            info!(
                "Loaded custom template '{}' from {:?}",
                template_id, template_path
            );
            Some(content)
        }
        Err(e) => {
            debug!("No custom template '{}' found: {}", template_id, e);
            None
        }
    }
}

/// Load and parse a template by identifier
///
/// This function implements a fallback strategy:
/// 1. Check user's custom templates directory
/// 2. Check bundled resources directory (app templates)
/// 3. Fall back to built-in embedded templates
/// 4. Return error if not found in any location
///
/// # Arguments
/// * `template_id` - Template identifier (e.g., "daily_standup", "standard_meeting")
///
/// # Returns
/// Parsed and validated Template struct
pub fn get_template(template_id: &str) -> Result<Template, String> {
    info!("Loading template: {}", template_id);

    // Try custom template first, then bundled, then built-in
    let json_content = if let Some(custom_content) = load_custom_template(template_id) {
        debug!("Using custom template for '{}'", template_id);
        custom_content
    } else if let Some(bundled_content) = load_bundled_template(template_id) {
        debug!("Using bundled template for '{}'", template_id);
        bundled_content
    } else if let Some(builtin_content) = defaults::get_builtin_template(template_id) {
        debug!("Using built-in template for '{}'", template_id);
        builtin_content.to_string()
    } else {
        return Err(format!(
            "Template '{}' not found. Available templates: {}",
            template_id,
            list_template_ids().join(", ")
        ));
    };

    // Parse and validate
    validate_and_parse_template(&json_content)
}

/// Validate and parse template JSON
///
/// # Arguments
/// * `json_content` - Raw JSON string
///
/// # Returns
/// Parsed and validated Template struct
pub fn validate_and_parse_template(json_content: &str) -> Result<Template, String> {
    let template: Template = serde_json::from_str(json_content)
        .map_err(|e| format!("Failed to parse template JSON: {}", e))?;

    template.validate()?;

    Ok(template)
}

/// List all available template identifiers
///
/// Returns a combined list of:
/// - Built-in template IDs
/// - Bundled template IDs (from app resources)
/// - Custom template IDs (from user's data directory)
pub fn list_template_ids() -> Vec<String> {
    let mut ids: Vec<String> = defaults::list_builtin_template_ids()
        .into_iter()
        .map(|s| s.to_string())
        .collect();

    // Add bundled templates if directory is set
    if let Ok(bundled_dir_lock) = BUNDLED_TEMPLATES_DIR.read() {
        if let Some(bundled_dir) = bundled_dir_lock.as_ref() {
            if bundled_dir.exists() {
                match std::fs::read_dir(bundled_dir) {
                    Ok(entries) => {
                        for entry in entries.flatten() {
                            if let Some(filename) = entry.file_name().to_str() {
                                if filename.ends_with(".json") {
                                    let id = filename.trim_end_matches(".json").to_string();
                                    if !ids.contains(&id) {
                                        ids.push(id);
                                    }
                                }
                            }
                        }
                    }
                    Err(e) => {
                        warn!("Failed to read bundled templates directory: {}", e);
                    }
                }
            }
        }
    }

    // Add custom templates if directory exists
    if let Some(custom_dir) = get_custom_templates_dir() {
        if custom_dir.exists() {
            match std::fs::read_dir(&custom_dir) {
                Ok(entries) => {
                    for entry in entries.flatten() {
                        if let Some(filename) = entry.file_name().to_str() {
                            // Dot-files are subsystem sidecars (e.g. the
                            // hidden-template list), not templates.
                            if filename.ends_with(".json") && !filename.starts_with('.') {
                                let id = filename.trim_end_matches(".json").to_string();
                                if !ids.contains(&id) {
                                    ids.push(id);
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    warn!("Failed to read custom templates directory: {}", e);
                }
            }
        }
    }

    ids.sort();
    ids
}

/// List all available templates with their metadata
///
/// Returns a list of (id, name, description) tuples
pub fn list_templates() -> Vec<(String, String, String)> {
    let mut templates = Vec::new();

    for id in list_template_ids() {
        match get_template(&id) {
            Ok(template) => {
                templates.push((id, template.name, template.description));
            }
            Err(e) => {
                warn!("Failed to load template '{}': {}", id, e);
            }
        }
    }

    templates
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_builtin_template() {
        let template = get_template("daily_standup");
        assert!(template.is_ok());

        let template = template.unwrap();
        assert_eq!(template.name, "Daily Standup");
        assert!(!template.sections.is_empty());
    }

    #[test]
    fn test_get_nonexistent_template() {
        let result = get_template("nonexistent_template");
        assert!(result.is_err());
    }

    #[test]
    fn test_list_template_ids() {
        let ids = list_template_ids();
        assert!(ids.contains(&"daily_standup".to_string()));
        assert!(ids.contains(&"standard_meeting".to_string()));
    }

    #[test]
    fn test_validate_invalid_json() {
        let result = validate_and_parse_template("invalid json");
        assert!(result.is_err());
    }

    #[test]
    fn test_classify_source() {
        // Custom-dir file, id not shipped with the app → user-created template.
        assert_eq!(classify_source(true, false), "custom");
        // Custom-dir file shadowing a built-in/bundled id → override.
        assert_eq!(classify_source(true, true), "override");
        // No custom file → shipped template (embedded or bundled dir).
        assert_eq!(classify_source(false, true), "builtin");
        assert_eq!(classify_source(false, false), "builtin");
    }

    #[test]
    fn test_builtin_ids_are_builtin_or_bundled() {
        // All five embedded built-ins count as shipped even with no bundled dir set.
        for id in defaults::list_builtin_template_ids() {
            assert!(is_builtin_or_bundled(id), "'{}' should be shipped", id);
        }
        assert!(!is_builtin_or_bundled("some_user_template"));
    }

    #[test]
    fn test_hidden_ids_round_trip() {
        let dir = tempfile::tempdir().expect("tempdir");
        let dir = dir.path();

        // Nothing hidden initially (no sidecar file).
        assert!(read_hidden_ids_from(dir).is_empty());

        // Hide two, restore one.
        set_hidden_in_dir(dir, "retrospective", true).expect("hide");
        set_hidden_in_dir(dir, "daily_standup", true).expect("hide 2");
        let ids = read_hidden_ids_from(dir);
        assert!(ids.contains("retrospective") && ids.contains("daily_standup"));

        set_hidden_in_dir(dir, "daily_standup", false).expect("restore");
        let ids = read_hidden_ids_from(dir);
        assert!(ids.contains("retrospective"));
        assert!(!ids.contains("daily_standup"));

        // No-op writes (already hidden / already visible) succeed.
        set_hidden_in_dir(dir, "retrospective", true).expect("re-hide no-op");
        set_hidden_in_dir(dir, "daily_standup", false).expect("re-restore no-op");
    }

    #[test]
    fn test_hidden_sidecar_survives_corruption_and_is_not_a_template() {
        let dir = tempfile::tempdir().expect("tempdir");
        let dir = dir.path();

        // Corrupt sidecar = nothing hidden, never an error.
        std::fs::write(dir.join(HIDDEN_TEMPLATES_FILE), "not json").expect("write");
        assert!(read_hidden_ids_from(dir).is_empty());
        // And a subsequent hide overwrites it cleanly.
        set_hidden_in_dir(dir, "daily_standup", true).expect("hide over corrupt file");
        assert!(read_hidden_ids_from(dir).contains("daily_standup"));
    }

    use crate::summary::outline::AUTO_TEMPLATE_ID;

    /// `auto` is a reserved id, never a file. Loading it must fail so nothing
    /// can shadow it with a custom template of the same name.
    #[test]
    fn auto_is_reserved_and_never_loads_as_a_template() {
        assert!(get_template(AUTO_TEMPLATE_ID).is_err());
    }

    /// It must not appear in the id list either — the picker adds Auto itself.
    #[test]
    fn auto_is_not_in_the_template_id_list() {
        assert!(!list_template_ids().contains(&AUTO_TEMPLATE_ID.to_string()));
    }

    /// A custom template must not be able to shadow the reserved id.
    /// `loader.rs` has no save function — writes go through
    /// `template_commands::api_save_template` (verified: loader.rs exposes
    /// get/list/validate/hide only), so the guard lives there and this asserts
    /// the reserved-id helper it calls.
    #[test]
    fn auto_is_recognised_as_a_reserved_id() {
        assert!(is_reserved_template_id(AUTO_TEMPLATE_ID));
        assert!(!is_reserved_template_id("standard_meeting"));
        assert!(!is_reserved_template_id("my_custom_one"));
    }
}
