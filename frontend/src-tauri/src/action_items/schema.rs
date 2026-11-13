//! The JSON schema action-item extraction asks for (specs/0085).
//!
//! Providers that can enforce a schema (the subscription CLIs: Claude Code's
//! `--json-schema`) get it through [`GenerationOptions::json_schema`]; HTTP providers ignore
//! the field and keep `response_format: json_object`. The shape is the object-wrapped one
//! the prompt already asks for (specs/0054 W1), in strict form: every property required,
//! no extra properties at any level. `reply_parse.rs` is unchanged: the re-serialized
//! structured output contains the array, and its bracket slice finds it.

use crate::summary::summary_engine::options::GenerationOptions;
use serde_json::{json, Value};

/// `{"action_items": [{description, assignee, due, context, same_as}]}`, strict.
pub fn action_items_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["action_items"],
        "properties": {
            "action_items": {
                "type": "array",
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["description", "assignee", "due", "context", "same_as"],
                    "properties": {
                        "description": {"type": "string"},
                        "assignee": {"type": ["string", "null"]},
                        "due": {"type": ["string", "null"]},
                        "context": {"type": ["string", "null"]},
                        "same_as": {"type": ["integer", "null"]}
                    }
                }
            }
        }
    })
}

/// Extraction's generation options: [`GenerationOptions::deterministic_json`] plus the
/// schema.
pub fn extraction_options() -> GenerationOptions {
    GenerationOptions {
        json_schema: Some(action_items_schema()),
        ..GenerationOptions::deterministic_json()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action_items::reply_parse::parse_candidates;
    use crate::summary::llm_client::wants_json_mode;

    /// Every object node: `additionalProperties: false` and every property required.
    fn assert_strict(node: &Value, path: &str) {
        if node["type"] == "object" {
            assert_eq!(node["additionalProperties"], false, "{path}");
            let mut props: Vec<&str> = node["properties"]
                .as_object()
                .unwrap_or_else(|| panic!("{path}: object without properties"))
                .keys()
                .map(String::as_str)
                .collect();
            let mut required: Vec<&str> = node["required"]
                .as_array()
                .unwrap_or_else(|| panic!("{path}: no required list"))
                .iter()
                .map(|v| v.as_str().unwrap())
                .collect();
            props.sort_unstable();
            required.sort_unstable();
            assert_eq!(props, required, "{path}: every property is required");
            for (name, child) in node["properties"].as_object().unwrap() {
                assert_strict(child, &format!("{path}.{name}"));
            }
        }
        if let Some(items) = node.get("items") {
            assert_strict(items, &format!("{path}[]"));
        }
    }

    #[test]
    fn the_schema_is_strict_at_every_level() {
        let schema = action_items_schema();
        assert_strict(&schema, "$");
        let item = &schema["properties"]["action_items"]["items"]["properties"];
        assert_eq!(item["description"]["type"], "string");
        assert_eq!(item["assignee"]["type"], json!(["string", "null"]));
        assert_eq!(item["due"]["type"], json!(["string", "null"]));
        assert_eq!(item["context"]["type"], json!(["string", "null"]));
        assert_eq!(item["same_as"]["type"], json!(["integer", "null"]));
    }

    #[test]
    fn extraction_options_add_the_schema_and_keep_json_mode() {
        let options = extraction_options();
        assert_eq!(options.json_schema, Some(action_items_schema()));
        // HTTP providers still get response_format: json_object, and nothing else changes.
        assert!(wants_json_mode(&options));
        assert_eq!(
            GenerationOptions {
                json_schema: None,
                ..options
            },
            GenerationOptions::deterministic_json()
        );
    }

    #[test]
    fn a_three_item_structured_output_parses_to_three_candidates() {
        // The specs/0054 regression shape: three items that json_object once collapsed
        // to one. The CLI path returns the structured output re-serialized like this.
        let reply = json!({"action_items": [
            {"description": "Send the revised launch checklist", "assignee": "me", "due": "Friday"},
            {"description": "Book the demo room", "assignee": null, "due": null},
            {"description": "Draft the release notes", "assignee": "me", "due": "next week"}
        ]})
        .to_string();
        let candidates = parse_candidates(&reply).unwrap();
        assert_eq!(candidates.len(), 3);
        assert_eq!(candidates[1].assignee, None);
    }
}
