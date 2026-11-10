//! The full access log as a plain-text file (specs/0087). Settings shows only the newest
//! few calls; "Open full log" writes everything the log still holds (90 days) here and
//! opens it. Regenerated on every open, deleted by "Clear log".

use super::log::AccessEntry;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub const FILE_NAME: &str = "agent_access_log.txt";

pub fn file_path(data_dir: &Path) -> PathBuf {
    data_dir.join(FILE_NAME)
}

fn tool_label(tool: &str) -> &str {
    match tool {
        "search_meetings" => "Searched",
        "list_meetings" => "Listed meetings",
        "get_meeting" => "Read a meeting",
        other => other,
    }
}

fn outcome_label(outcome: &str) -> Option<&'static str> {
    match outcome {
        "ok" => None,
        "off" => Some("refused, access was off"),
        "needs_update" => Some("refused, Nixon needed updating"),
        "no_db" => Some("refused, no meetings yet"),
        _ => Some("error"),
    }
}

/// `key=value` for every non-null argument, in the order the log stored them.
fn describe_args(args: &serde_json::Value) -> String {
    let Some(map) = args.as_object() else {
        return String::new();
    };
    map.iter()
        .filter(|(_, v)| !v.is_null())
        .map(|(k, v)| match v {
            serde_json::Value::String(s) => format!("{k}=\"{s}\""),
            other => format!("{k}={other}"),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// RFC 3339 UTC → local `YYYY-MM-DD HH:MM`; an unparseable time is shown as stored.
fn local_time(at: &str) -> String {
    chrono::DateTime::parse_from_rfc3339(at)
        .map(|t| {
            t.with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M")
                .to_string()
        })
        .unwrap_or_else(|_| at.to_string())
}

/// One block per call, newest first. `titles` maps meeting id → title (missing = deleted).
pub fn format(entries: &[AccessEntry], titles: &HashMap<String, Option<String>>) -> String {
    let mut out = String::from(
        "Nixon agent access log: every call Claude made, newest first (kept for 90 days).\n\n",
    );
    if entries.is_empty() {
        out.push_str("Nothing has read your meetings yet.\n");
        return out;
    }
    for e in entries {
        out.push_str(&local_time(&e.at));
        out.push_str("  ");
        out.push_str(tool_label(&e.tool));
        if let Some(c) = e.client.as_deref().filter(|c| !c.is_empty()) {
            out.push_str(&format!("  (by {c})"));
        }
        if let Some(o) = outcome_label(&e.outcome) {
            out.push_str(&format!("  [{o}]"));
        }
        out.push('\n');
        let args = describe_args(&e.args);
        if !args.is_empty() {
            out.push_str(&format!("    {args}\n"));
        }
        for id in &e.meeting_ids {
            let title = match titles.get(id) {
                Some(Some(t)) => t.as_str(),
                Some(None) => "Untitled",
                None => "deleted meeting",
            };
            out.push_str(&format!("    {title}  ({id})\n"));
        }
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn entry(tool: &str, outcome: &str, args: serde_json::Value, ids: &[&str]) -> AccessEntry {
        AccessEntry {
            at: "2026-11-09T21:14:00Z".into(),
            client: Some("claude-ai".into()),
            tool: tool.into(),
            args,
            meeting_ids: ids.iter().map(|s| s.to_string()).collect(),
            outcome: outcome.into(),
        }
    }

    #[test]
    fn empty_log_says_so() {
        assert!(format(&[], &HashMap::new()).contains("Nothing has read your meetings yet."));
    }

    #[test]
    fn lists_terms_meetings_client_and_refusals() {
        let titles = HashMap::from([
            ("m1".to_string(), Some("Product sync".to_string())),
            ("m2".to_string(), None),
        ]);
        let text = format(
            &[
                entry(
                    "search_meetings",
                    "ok",
                    json!({"query": "firmware", "limit": 2, "person": null}),
                    &["m1", "m2", "gone"],
                ),
                entry("get_meeting", "off", json!({"id": "m1"}), &[]),
            ],
            &titles,
        );
        assert!(text.contains("Searched  (by claude-ai)\n"), "{text}");
        // serde_json maps are sorted, so arguments come out alphabetically.
        assert!(text.contains("    limit=2, query=\"firmware\"\n"), "{text}");
        assert!(!text.contains("person"), "{text}");
        assert!(text.contains("    Product sync  (m1)\n"));
        assert!(text.contains("    Untitled  (m2)\n"));
        assert!(text.contains("    deleted meeting  (gone)\n"));
        assert!(text.contains("Read a meeting  (by claude-ai)  [refused, access was off]\n"));
        // Newest first is the caller's order; the file keeps it.
        assert!(text.find("Searched").unwrap() < text.find("Read a meeting").unwrap());
    }

    #[test]
    fn unparseable_time_is_kept_verbatim() {
        let mut e = entry("list_meetings", "ok", json!({}), &[]);
        e.at = "yesterday".into();
        assert!(format(&[e], &HashMap::new()).contains("yesterday  Listed meetings"));
    }
}
