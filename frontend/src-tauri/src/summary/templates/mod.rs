//! Meeting summary template management
//!
//! This module provides a flexible template system for generating meeting summaries.
//! It supports both built-in templates (embedded in the binary) and custom user templates
//! (loaded from the application data directory).
//!
//! # Architecture
//!
//! - **Built-in templates**: JSON files in `frontend/src-tauri/templates/` embedded at compile time
//! - **Custom templates**: JSON files in platform-specific app data directory
//! - **Fallback strategy**: Custom templates override built-in templates with the same ID
//!
//! # Usage
//!
//! ```rust
//! use app_lib::summary::templates;
//!
//! // Load a specific template
//! let template = templates::get_template("daily_standup").expect("daily standup template exists");
//!
//! // Generate markdown structure
//! let markdown = template.to_markdown_structure();
//!
//! // Generate LLM instructions
//! let instructions = template.to_section_instructions();
//!
//! // List available templates
//! let available = templates::list_templates();
//! ```
//!
//! # Custom Templates
//!
//! Users can add custom templates to the `templates/` subdirectory of the app's
//! identifier-derived data directory (`~/Library/Application Support/<bundle-id>/`
//! on macOS). See `crate::app_paths`.
//!
//! Custom templates must follow the JSON schema defined in `types::Template`.

mod defaults;
mod loader;
mod types;

/// The default *template file* id. This is NOT the id a new meeting's
/// `template_id` should resolve to (see [`DEFAULT_SUMMARY_TEMPLATE_ID`] for
/// that) — it exists so [`get_template`] always has a real fixed-template
/// fallback to load (see `test_default_template_id_resolves` below), and it
/// backs `summary::outline::FALLBACK_TEMPLATE_ID`, the shape Auto degrades to
/// when derivation fails. "auto" is deliberately never a loadable file, so it
/// must never be assigned here.
pub const DEFAULT_TEMPLATE_ID: &str = "standard_meeting";

/// The default used at the two NULL-`meetings.template_id` resolution sites
/// in `summary::commands` (specs/0053 C1 fix), matching the frontend default
/// in `hooks/meeting-details/useTemplates.ts`. Points at
/// `summary::outline::AUTO_TEMPLATE_ID` rather than [`DEFAULT_TEMPLATE_ID`]
/// because "auto" is a reserved dispatch id, not a template file — resolving
/// it through `get_template` would fail.
pub const DEFAULT_SUMMARY_TEMPLATE_ID: &str = crate::summary::outline::AUTO_TEMPLATE_ID;

// Re-export public API
pub use loader::{
    get_custom_templates_dir, get_template, hidden_template_ids, is_builtin_or_bundled,
    is_reserved_template_id, list_template_ids, list_templates, set_bundled_templates_dir,
    set_template_hidden, template_source, validate_and_parse_template,
};
pub use types::{Template, TemplateSection};

// Crate-internal: pure source classification, unit-testable without touching
// the real custom-templates dir (see summary/template_commands.rs tests).
#[cfg(test)]
pub(crate) use loader::classify_source;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_module_integration() {
        // Test that we can load all built-in templates
        let ids = list_template_ids();
        assert!(!ids.is_empty());

        for id in ids {
            let result = get_template(&id);
            assert!(
                result.is_ok(),
                "Failed to load template '{}': {:?}",
                id,
                result.err()
            );
        }
    }

    #[test]
    fn test_default_template_id_resolves() {
        let template = get_template(DEFAULT_TEMPLATE_ID)
            .expect("the shared default template must always resolve");
        assert!(!template.sections.is_empty());
    }

    #[test]
    fn test_template_metadata() {
        let templates = list_templates();
        assert!(!templates.is_empty());

        for (id, name, description) in templates {
            assert!(!id.is_empty());
            assert!(!name.is_empty());
            assert!(!description.is_empty());
        }
    }
}
