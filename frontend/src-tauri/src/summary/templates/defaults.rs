// Embedded default templates using compile-time inclusion
//
// These templates are bundled into the binary and serve as fallbacks
// when custom templates are not available.

/// Daily standup template for engineering/product teams
pub const DAILY_STANDUP: &str = include_str!("../../../templates/daily_standup.json");

/// Standard meeting notes template
pub const STANDARD_MEETING: &str = include_str!("../../../templates/standard_meeting.json");

/// Project sync template for status/blockers/decisions
pub const PROJECT_SYNC: &str = include_str!("../../../templates/project_sync.json");

/// Retrospective template (went well / didn't / actions)
pub const RETROSPECTIVE: &str = include_str!("../../../templates/retrospective.json");

/// Sales / marketing / client call template
pub const SALES_MARKETING_CLIENT_CALL: &str =
    include_str!("../../../templates/sales_marketing_client_call.json");

/// Psychiatric session (SOAP) template. NOTE: the id keeps the historical
/// misspelling "psychatric_session" for back-compat with persisted
/// `meetings.template_id` values; only the display name inside the JSON was
/// fixed (specs/0020 task 2).
pub const PSYCHATRIC_SESSION: &str = include_str!("../../../templates/psychatric_session.json");

/// Registry of all built-in templates
///
/// Maps template identifiers to their embedded JSON content
pub fn get_builtin_templates() -> Vec<(&'static str, &'static str)> {
    vec![
        ("daily_standup", DAILY_STANDUP),
        ("standard_meeting", STANDARD_MEETING),
        ("project_sync", PROJECT_SYNC),
        ("retrospective", RETROSPECTIVE),
        ("sales_marketing_client_call", SALES_MARKETING_CLIENT_CALL),
        ("psychatric_session", PSYCHATRIC_SESSION),
    ]
}

/// Get a built-in template by identifier
///
/// # Arguments
/// * `id` - Template identifier (e.g., "daily_standup", "standard_meeting")
///
/// # Returns
/// The template JSON content if found, None otherwise
pub fn get_builtin_template(id: &str) -> Option<&'static str> {
    get_builtin_templates()
        .into_iter()
        .find(|(builtin_id, _)| *builtin_id == id)
        .map(|(_, content)| content)
}

/// List all built-in template identifiers
pub fn list_builtin_template_ids() -> Vec<&'static str> {
    get_builtin_templates()
        .into_iter()
        .map(|(id, _)| id)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_builtin_templates_parse_and_validate() {
        let templates = get_builtin_templates();
        assert_eq!(templates.len(), 6, "expected all six built-in templates");

        for (id, content) in templates {
            let template: super::super::types::Template = serde_json::from_str(content)
                .unwrap_or_else(|e| panic!("Built-in template '{}' failed to parse: {}", id, e));
            template
                .validate()
                .unwrap_or_else(|e| panic!("Built-in template '{}' failed validation: {}", id, e));
        }
    }

    #[test]
    fn test_get_builtin_template() {
        assert!(get_builtin_template("daily_standup").is_some());
        assert!(get_builtin_template("standard_meeting").is_some());
        assert!(get_builtin_template("project_sync").is_some());
        assert!(get_builtin_template("retrospective").is_some());
        assert!(get_builtin_template("sales_marketing_client_call").is_some());
        assert!(get_builtin_template("psychatric_session").is_some());
        assert!(get_builtin_template("nonexistent").is_none());
    }

    #[test]
    fn test_list_builtin_template_ids_contains_all_six() {
        let ids = list_builtin_template_ids();
        assert_eq!(ids.len(), 6);
        for id in [
            "daily_standup",
            "standard_meeting",
            "project_sync",
            "retrospective",
            "sales_marketing_client_call",
            "psychatric_session",
        ] {
            assert!(ids.contains(&id), "missing built-in id '{}'", id);
        }
    }

    #[test]
    fn test_psychiatric_display_name_fixed() {
        let template: super::super::types::Template =
            serde_json::from_str(PSYCHATRIC_SESSION).expect("psychatric_session parses");
        assert_eq!(template.name, "Psychiatric Session");
    }
}
