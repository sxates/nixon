//! `@person` mention tokens in action-item titles (specs/0088).
//!
//! A mention is stored inline in `description` as `@[Display Name](person:<person_id>)`.
//! Everything that leaves the row as text (LLM prompt, similarity, MCP) goes through
//! [`plain_description`], so the token never leaks.

use std::sync::OnceLock;

use regex::Regex;

fn token_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"@\[([^\]]+)\]\(person:([^)]+)\)").expect("valid mention regex"))
}

/// Replace every `@[Name](person:id)` token with `Name`.
pub fn plain_description(desc: &str) -> String {
    token_re().replace_all(desc, "$1").into_owned()
}

/// Person ids mentioned in `desc`, de-duplicated, in first-seen order.
pub fn person_ids_in(desc: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for cap in token_re().captures_iter(desc) {
        let id = cap[2].trim().to_string();
        if !id.is_empty() && !out.contains(&id) {
            out.push(id);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_strips_tokens() {
        assert_eq!(
            plain_description("Check in with @[Sam Lee](person:p-1) re @[Ana](person:p-2)"),
            "Check in with Sam Lee re Ana"
        );
    }

    #[test]
    fn ids_dedup_in_order() {
        assert_eq!(
            person_ids_in("@[B](person:p-2) and @[A](person:p-1) and @[B again](person:p-2)"),
            vec!["p-2".to_string(), "p-1".to_string()]
        );
    }

    #[test]
    fn bare_at_is_plain_text() {
        assert_eq!(plain_description("email @team"), "email @team");
        assert!(person_ids_in("email @team").is_empty());
    }

    #[test]
    fn content_key_ignores_token_syntax() {
        use crate::action_items::diff::content_key;
        assert_eq!(
            content_key("Ping @[Sam Lee](person:p-1) today"),
            content_key("Ping Sam Lee today")
        );
    }
}
