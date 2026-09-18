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
        assert_eq!(templates.len(), 5, "expected all five built-in templates");

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
        assert!(get_builtin_template("nonexistent").is_none());
    }

    #[test]
    fn test_list_builtin_template_ids_contains_all_five() {
        let ids = list_builtin_template_ids();
        assert_eq!(ids.len(), 5);
        for id in [
            "daily_standup",
            "standard_meeting",
            "project_sync",
            "retrospective",
            "sales_marketing_client_call",
        ] {
            assert!(ids.contains(&id), "missing built-in id '{}'", id);
        }
    }
}
