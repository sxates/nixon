use crate::database::repositories::meeting::MeetingsRepository;
use crate::state::AppState;
use crate::summary::templates;
use crate::summary::templates::{Template, TemplateSection};
use serde::{Deserialize, Serialize};
use std::path::Path;
use tauri::Runtime;
use tracing::{info, warn};

/// Template metadata for UI display
#[derive(Debug, Serialize, Deserialize)]
pub struct TemplateInfo {
    /// Template identifier (e.g., "daily_standup", "standard_meeting")
    pub id: String,

    /// Display name for the template
    pub name: String,

    /// Brief description of the template's purpose
    pub description: String,

    /// Where the template resolves from: "builtin" (shipped, including
    /// bundled-dir templates), "custom" (user-created), or "override"
    /// (a custom file shadowing a shipped id). specs/0020 task 5.
    pub source: String,

    /// True when the user removed this template from their list (shipped
    /// templates are hidden, not deleted — restorable from settings; the id
    /// still resolves for meetings that persisted it).
    pub hidden: bool,
}

/// Detailed template structure for preview/debugging and the settings editor
#[derive(Debug, Serialize, Deserialize)]
pub struct TemplateDetails {
    /// Template identifier
    pub id: String,

    /// Display name
    pub name: String,

    /// Description
    pub description: String,

    /// List of section titles in order (kept for existing callers)
    pub sections: Vec<String>,

    /// Full ordered sections (title/instruction/format/item_format/
    /// example_item_format) so the settings editor can round-trip a template
    /// (specs/0020 task 5).
    pub section_details: Vec<TemplateSection>,
}

/// Result of [`api_save_template`]: the id the template was persisted under
/// (either the caller's or one slugified from the template name).
#[derive(Debug, Serialize, Deserialize)]
pub struct SavedTemplate {
    pub id: String,
    pub name: String,
}

/// Derives a filesystem-safe template id from a display name: lowercase,
/// non-alphanumeric runs collapsed to single `_`, leading/trailing `_` trimmed.
fn slugify_template_name(name: &str) -> Result<String, String> {
    let mut slug = String::new();
    let mut pending_separator = false;
    for c in name.to_lowercase().chars() {
        if c.is_ascii_alphanumeric() {
            if pending_separator && !slug.is_empty() {
                slug.push('_');
            }
            slug.push(c);
            pending_separator = false;
        } else {
            pending_separator = true;
        }
    }

    if slug.is_empty() {
        Err(format!(
            "Cannot derive a template id from the name '{}'. Use letters or digits in the template name.",
            name
        ))
    } else {
        Ok(slug)
    }
}

/// Rejects ids that could escape the custom templates directory. Ids become
/// `<custom_dir>/<id>.json`, so path separators and dots are never legal.
fn validate_template_id(id: &str) -> Result<(), String> {
    if id.is_empty() {
        return Err("Template id cannot be empty".to_string());
    }
    if id.contains('/') || id.contains('\\') || id.contains('.') {
        return Err(format!(
            "Invalid template id '{}': ids cannot contain '/', '\\', or '.'",
            id
        ));
    }
    Ok(())
}

/// Writes a validated template as pretty JSON to `<dir>/<id>.json`, creating
/// the directory if needed. Split out from the command so tests can target a
/// temp dir instead of the real app-data custom dir.
fn save_template_to_dir(dir: &Path, id: &str, template: &Template) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| {
        format!(
            "Failed to create the custom templates directory {:?}: {}",
            dir, e
        )
    })?;

    let json = serde_json::to_string_pretty(template)
        .map_err(|e| format!("Failed to serialize template '{}': {}", id, e))?;

    let path = dir.join(format!("{}.json", id));
    std::fs::write(&path, json)
        .map_err(|e| format!("Failed to write template '{}' to {:?}: {}", id, path, e))?;

    info!("Saved template '{}' to {:?}", id, path);
    Ok(())
}

/// Deletes `<dir>/<id>.json` (the custom copy only). `shadows_shipped` marks
/// ids that ship with the app: with no custom file present those are
/// undeletable built-ins, not missing files.
fn delete_template_from_dir(dir: &Path, id: &str, shadows_shipped: bool) -> Result<(), String> {
    let path = dir.join(format!("{}.json", id));

    if path.is_file() {
        std::fs::remove_file(&path)
            .map_err(|e| format!("Failed to delete template '{}' at {:?}: {}", id, path, e))?;
        info!("Deleted custom template '{}' at {:?}", id, path);
        Ok(())
    } else if shadows_shipped {
        Err("built-in templates cannot be deleted".to_string())
    } else {
        Err(format!("Template '{}' not found", id))
    }
}

/// Lists all available templates
///
/// Returns templates from both built-in (embedded) and custom (user data directory) sources.
/// Templates are automatically discovered - no code changes needed to add new templates.
///
/// # Returns
/// Vector of TemplateInfo with id, name, and description for each template
#[tauri::command]
pub async fn api_list_templates<R: Runtime>(
    _app: tauri::AppHandle<R>,
) -> Result<Vec<TemplateInfo>, String> {
    info!("api_list_templates called");

    let templates = templates::list_templates();
    let hidden_ids = templates::hidden_template_ids();

    let template_infos: Vec<TemplateInfo> = templates
        .into_iter()
        .map(|(id, name, description)| {
            let source = templates::template_source(&id).to_string();
            let hidden = hidden_ids.contains(&id);
            TemplateInfo {
                id,
                name,
                description,
                source,
                hidden,
            }
        })
        .collect();

    info!("Found {} available templates", template_infos.len());

    Ok(template_infos)
}

/// Gets detailed information about a specific template
///
/// # Arguments
/// * `template_id` - Template identifier (e.g., "daily_standup")
///
/// # Returns
/// TemplateDetails with full template structure
#[tauri::command]
pub async fn api_get_template_details<R: Runtime>(
    _app: tauri::AppHandle<R>,
    template_id: String,
) -> Result<TemplateDetails, String> {
    info!(
        "api_get_template_details called for template_id: {}",
        template_id
    );

    let template = templates::get_template(&template_id)?;

    let section_titles: Vec<String> = template
        .sections
        .iter()
        .map(|section| section.title.clone())
        .collect();

    let details = TemplateDetails {
        id: template_id,
        name: template.name,
        description: template.description,
        sections: section_titles,
        section_details: template.sections,
    };

    info!("Retrieved template details for '{}'", details.name);

    Ok(details)
}

/// Validates a custom template JSON string
///
/// Useful for template editor UI or validation before saving custom templates
///
/// # Arguments
/// * `template_json` - Raw JSON string of the template
///
/// # Returns
/// Ok(template_name) if valid, Err(error_message) if invalid
#[tauri::command]
pub async fn api_validate_template<R: Runtime>(
    _app: tauri::AppHandle<R>,
    template_json: String,
) -> Result<String, String> {
    info!("api_validate_template called");

    match templates::validate_and_parse_template(&template_json) {
        Ok(template) => {
            info!("Template '{}' validated successfully", template.name);
            Ok(template.name)
        }
        Err(e) => {
            warn!("Template validation failed: {}", e);
            Err(e)
        }
    }
}

/// Reads the persisted per-meeting summary template (specs/0029 WS4.3 — the
/// specs/0020 `meetings.template_id` persistence slice).
///
/// Returns `None` when the meeting carries no explicit choice. Both sides then
/// fall back to the same default — the reserved `auto` id (specs/0053 W3):
/// the picker via `useTemplates`' `DEFAULT_TEMPLATE_ID`, and generation via
/// [`crate::summary::templates::DEFAULT_SUMMARY_TEMPLATE_ID`]. (This comment used
/// to say "standard_meeting"; that was stale from before 0053 and sent the 0054
/// investigation down a false trail.)
#[tauri::command]
pub async fn api_get_meeting_template<R: Runtime>(
    _app: tauri::AppHandle<R>,
    state: tauri::State<'_, AppState>,
    meeting_id: String,
) -> Result<Option<String>, String> {
    info!(
        "api_get_meeting_template called for meeting_id: {}",
        meeting_id
    );

    let pool = state.db_manager.pool();
    MeetingsRepository::get_meeting_template(pool, &meeting_id)
        .await
        .map_err(|e| {
            warn!("Failed to read template for meeting {}: {}", meeting_id, e);
            format!("Failed to read meeting template: {}", e)
        })
}

/// Validate a non-empty per-meeting template choice (specs/0054 W2a).
///
/// Reserved ids (`auto`) are deliberately NOT files on disk — `get_template`
/// always errors on them — so validating with it alone rejected every attempt to
/// persist Auto, which is what produced the owner's "Could not save template
/// choice" toast. Reserved ids are legitimate choices that `summary/service.rs`
/// resolves at generation time, so they bypass the file lookup; everything else
/// must still resolve to a real shipped or custom template.
fn template_choice_is_valid(id: &str) -> Result<(), String> {
    if templates::is_reserved_template_id(id) {
        return Ok(());
    }
    templates::get_template(id)
        .map(|_| ())
        .map_err(|e| format!("Unknown template '{}': {}", id, e))
}

/// Persists the summary template choice for one meeting (specs/0029 WS4.3).
///
/// `template_id = None` (or blank) clears the choice back to "use the default".
/// Non-empty ids are validated against the known built-in/custom templates (plus
/// the reserved ids — specs/0054 W2a) so a stale or mistyped id can't be stored
/// and break summary generation later.
#[tauri::command]
pub async fn api_set_meeting_template<R: Runtime>(
    _app: tauri::AppHandle<R>,
    state: tauri::State<'_, AppState>,
    meeting_id: String,
    template_id: Option<String>,
) -> Result<(), String> {
    info!(
        "api_set_meeting_template called for meeting_id: {}, template_id: {:?}",
        meeting_id, template_id
    );

    if let Some(id) = template_id
        .as_deref()
        .map(str::trim)
        .filter(|t| !t.is_empty())
    {
        template_choice_is_valid(id).map_err(|e| {
            warn!("Rejecting template '{}': {}", id, e);
            e
        })?;
    }

    let pool = state.db_manager.pool();
    match MeetingsRepository::set_meeting_template(pool, &meeting_id, template_id.as_deref()).await
    {
        Ok(true) => {
            info!("Saved template choice for meeting {}", meeting_id);
            Ok(())
        }
        Ok(false) => {
            warn!("No meeting found with id {}", meeting_id);
            Err(format!("No meeting found with id {}", meeting_id))
        }
        Err(e) => {
            warn!("Failed to save template for meeting {}: {}", meeting_id, e);
            Err(format!("Failed to save meeting template: {}", e))
        }
    }
}

/// Saves (creates or edits) a template to the user's custom templates directory
/// (specs/0020 task 5).
///
/// `id = None` derives the id from the template's `name` (slugified). Saving
/// under an id that matches a built-in is deliberate: the loader's custom-first
/// precedence turns it into an **override**; deleting the custom file later
/// reverts to the shipped version.
#[tauri::command]
pub async fn api_save_template<R: Runtime>(
    _app: tauri::AppHandle<R>,
    id: Option<String>,
    template_json: String,
) -> Result<SavedTemplate, String> {
    info!("api_save_template called (id: {:?})", id);

    let template = templates::validate_and_parse_template(&template_json).map_err(|e| {
        warn!("api_save_template rejected invalid template: {}", e);
        e
    })?;

    let id = match id.map(|s| s.trim().to_string()).filter(|s| !s.is_empty()) {
        Some(explicit) => {
            validate_template_id(&explicit)?;
            explicit
        }
        None => slugify_template_name(&template.name)?,
    };

    // specs/0053 W3: `auto` is reserved for content-derived structure and is never
    // a file on disk — a custom template saved under that id would silently
    // shadow it in `get_template`'s custom-first lookup. Checked on the FINAL
    // resolved id (not just an explicit id param) so a template merely NAMED
    // "Auto" can't slip in through the slugify-from-name path either.
    if templates::is_reserved_template_id(&id) {
        return Err(format!("'{}' is a reserved template id", id));
    }

    let dir = templates::get_custom_templates_dir()
        .ok_or_else(|| "Could not resolve the custom templates directory".to_string())?;
    save_template_to_dir(&dir, &id, &template)?;

    Ok(SavedTemplate {
        id,
        name: template.name,
    })
}

/// Deletes a **custom** template file (specs/0020 task 5). Deleting an override
/// reverts the id to its shipped built-in; shipped ids without an override
/// cannot be deleted. Meetings whose persisted `template_id` dangles afterwards
/// fall back to the default at generation time — no cleanup pass needed.
#[tauri::command]
pub async fn api_delete_template<R: Runtime>(
    _app: tauri::AppHandle<R>,
    id: String,
) -> Result<(), String> {
    info!("api_delete_template called for id: {}", id);

    let id = id.trim();
    validate_template_id(id)?;

    let dir = templates::get_custom_templates_dir()
        .ok_or_else(|| "Could not resolve the custom templates directory".to_string())?;
    delete_template_from_dir(&dir, id, templates::is_builtin_or_bundled(id))
}

/// Hides or restores a template in the user's list (specs/0020 follow-up:
/// shipped templates the user will never use — e.g. Psychiatric Session — are
/// removable, but as a restorable *hide*, never a delete). Hidden ids still
/// resolve at generation time, so meetings that persisted one keep working;
/// they just stop appearing in pickers and the default settings list.
#[tauri::command]
pub async fn api_set_template_hidden<R: Runtime>(
    _app: tauri::AppHandle<R>,
    id: String,
    hidden: bool,
) -> Result<(), String> {
    info!(
        "api_set_template_hidden called (id: {}, hidden: {})",
        id, hidden
    );

    let id = id.trim();
    validate_template_id(id)?;
    // Hiding something unknown would strand an inert entry in the sidecar file;
    // restoring is allowed unconditionally so a stale entry can always be cleared.
    if hidden && templates::get_template(id).is_err() {
        return Err(format!("Template '{}' not found", id));
    }
    templates::set_template_hidden(id, hidden)
}

/// Suggests a template for a (new) meeting from its title (specs/0020 task 6):
/// the persisted template of the most recent prior meeting with the same
/// normalized title, or `None` when there is no such meeting.
#[tauri::command]
pub async fn api_suggest_template_for_title<R: Runtime>(
    _app: tauri::AppHandle<R>,
    state: tauri::State<'_, AppState>,
    title: String,
    exclude_meeting_id: Option<String>,
) -> Result<Option<String>, String> {
    info!(
        "api_suggest_template_for_title called (title: {:?}, exclude: {:?})",
        title, exclude_meeting_id
    );

    let pool = state.db_manager.pool();
    MeetingsRepository::suggest_template_for_title(pool, &title, exclude_meeting_id.as_deref())
        .await
        .map_err(|e| {
            warn!("Failed to suggest a template for title {:?}: {}", title, e);
            format!("Failed to look up a template suggestion: {}", e)
        })
}

/// Clear a meeting's derived Auto outline so the next summary re-derives its
/// structure ("Re-think structure", specs/0053 W3). No-op for a meeting that
/// never used Auto.
#[tauri::command]
pub async fn api_clear_summary_outline(
    state: tauri::State<'_, AppState>,
    meeting_id: String,
) -> Result<(), String> {
    info!(
        "api_clear_summary_outline called (meeting_id: {})",
        meeting_id
    );
    crate::database::repositories::summary_outline::SummaryOutlineRepository::delete(
        state.db_manager.pool(),
        &meeting_id,
    )
    .await
    .map_err(|e| {
        warn!("Failed to clear the outline for {}: {:#}", meeting_id, e);
        format!("Failed to clear the summary structure: {}", e)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// specs/0054 W2a: `auto` is a RESERVED id that deliberately has no file on
    /// disk (`templates/loader.rs`), so validating a choice with `get_template`
    /// rejected every attempt to persist it — the owner's "Could not save
    /// template choice" toast on every Auto selection. Reserved ids are
    /// legitimate choices (`summary/service.rs` resolves them at generation
    /// time) and must bypass the file-existence check.
    #[test]
    fn reserved_template_ids_are_accepted_without_a_file() {
        assert!(
            template_choice_is_valid("auto").is_ok(),
            "the reserved `auto` id must be persistable"
        );
        assert!(
            template_choice_is_valid("standard_meeting").is_ok(),
            "a real shipped template must still validate"
        );
        assert!(
            template_choice_is_valid("no_such_template_xyz").is_err(),
            "an unknown id must still be rejected"
        );
    }

    #[tokio::test]
    async fn test_list_templates() {
        // This test requires the templates to be embedded/available
        // In a real test environment, you might want to mock the templates module

        // For now, just verify the function compiles and runs
        // You can expand this with more specific assertions
    }

    #[tokio::test]
    async fn test_validate_template_valid() {
        let valid_json = r#"
        {
            "name": "Test Template",
            "description": "A test template",
            "sections": [
                {
                    "title": "Summary",
                    "instruction": "Provide a summary",
                    "format": "paragraph"
                }
            ]
        }"#;

        // Mock app handle would be needed for actual testing
        // For now, test the validation logic directly
        let result = templates::validate_and_parse_template(valid_json);
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_validate_template_invalid() {
        let invalid_json = "invalid json";

        let result = templates::validate_and_parse_template(invalid_json);
        assert!(result.is_err());
    }

    fn sample_template(name: &str) -> Template {
        Template {
            name: name.to_string(),
            description: "A test template".to_string(),
            sections: vec![TemplateSection {
                title: "Summary".to_string(),
                instruction: "Provide a summary".to_string(),
                format: "paragraph".to_string(),
                item_format: None,
                example_item_format: None,
            }],
        }
    }

    #[test]
    fn test_slugify_template_name() {
        assert_eq!(slugify_template_name("My Template").unwrap(), "my_template");
        assert_eq!(
            slugify_template_name("  Weekly Sync — Q3! ").unwrap(),
            "weekly_sync_q3"
        );
        // Repeated separators collapse; leading/trailing separators trim.
        assert_eq!(slugify_template_name("a--b__c").unwrap(), "a_b_c");
        assert_eq!(slugify_template_name("__Standup__").unwrap(), "standup");
        // Uppercase folds.
        assert_eq!(slugify_template_name("SOAP Note").unwrap(), "soap_note");
        // No usable characters → error.
        assert!(slugify_template_name("").is_err());
        assert!(slugify_template_name("— — !!").is_err());
    }

    #[test]
    fn test_validate_template_id_traversal_guard() {
        assert!(validate_template_id("my_template").is_ok());
        assert!(validate_template_id("standard_meeting").is_ok());
        assert!(validate_template_id("").is_err());
        assert!(validate_template_id("../evil").is_err());
        assert!(validate_template_id("a/b").is_err());
        assert!(validate_template_id("a\\b").is_err());
        assert!(validate_template_id("evil.json").is_err());
    }

    /// specs/0020 task 5: save/delete against an isolated dir (the commands use
    /// the app-data custom dir; the helpers are dir-parameterized so tests never
    /// touch a real install).
    #[test]
    fn test_save_delete_and_source_classification() {
        use crate::summary::templates::{is_builtin_or_bundled, validate_and_parse_template};

        let dir = tempfile::tempdir().expect("tempdir");
        let dir = dir.path();

        // Save a brand-new template → file exists, round-trips, source=custom.
        let template = sample_template("My Custom Template");
        save_template_to_dir(dir, "my_custom_template", &template).unwrap();
        let path = dir.join("my_custom_template.json");
        assert!(path.is_file());
        let reloaded =
            validate_and_parse_template(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(reloaded.name, "My Custom Template");
        assert!(!is_builtin_or_bundled("my_custom_template"));
        // Custom-dir file + non-shipped id → "custom".
        assert_eq!(
            crate::summary::templates::classify_source(true, false),
            "custom"
        );

        // Save under a built-in id → an override (custom file shadowing shipped).
        let edited = sample_template("Standard Meeting (edited)");
        save_template_to_dir(dir, "standard_meeting", &edited).unwrap();
        assert!(dir.join("standard_meeting.json").is_file());
        assert!(is_builtin_or_bundled("standard_meeting"));
        assert_eq!(
            crate::summary::templates::classify_source(true, true),
            "override"
        );

        // Delete the override → file gone, id reverts to the shipped built-in.
        delete_template_from_dir(dir, "standard_meeting", true).unwrap();
        assert!(!dir.join("standard_meeting.json").exists());
        let builtin = templates::get_template("standard_meeting").unwrap();
        assert_ne!(builtin.name, "Standard Meeting (edited)");
        assert_eq!(
            crate::summary::templates::classify_source(false, true),
            "builtin"
        );

        // Deleting a built-in with no override is refused.
        let err = delete_template_from_dir(dir, "standard_meeting", true).unwrap_err();
        assert!(
            err.contains("built-in templates cannot be deleted"),
            "{err}"
        );

        // Deleting an id with no file anywhere is not-found.
        let err = delete_template_from_dir(dir, "no_such_template", false).unwrap_err();
        assert!(err.contains("not found"), "{err}");

        // The plain custom template still deletes cleanly.
        delete_template_from_dir(dir, "my_custom_template", false).unwrap();
        assert!(!path.exists());
    }
}
