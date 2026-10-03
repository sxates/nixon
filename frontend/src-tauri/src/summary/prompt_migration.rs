//! One-time conversion of the user's summary templates into saved prompts (specs/0079 W3).
//!
//! Runs at startup and is idempotent via the `templates_converted` marker, which is written
//! last. Failures converting a single template are logged (ids only) and skipped.

use std::collections::{BTreeSet, HashMap};

use anyhow::Result;
use sqlx::SqlitePool;
use tracing::warn;

use crate::database::repositories::summary_prompt::SummaryPromptRepository;
use crate::summary::prompt_sanitize::{
    sanitize_prompt_body, sanitize_prompt_name, MAX_PROMPT_CHARS,
};
use crate::summary::templates::{get_template, hidden_template_ids, list_template_ids, Template};

pub const AUTO_PRESET_NAME: &str = "Let the model choose the structure";
/// Same text as the resolver's fallback, so the two cannot drift.
pub const AUTO_PRESET_BODY: &str = crate::summary::prompts_resolve::FALLBACK_PROMPT;

const AUTO_ID: &str = "auto";
const MARKER: &str = "templates_converted";

#[derive(Debug, Default, Clone, PartialEq)]
pub struct ConversionReport {
    pub prompts_created: usize,
    pub meetings_mapped: usize,
    pub skipped: Vec<String>,
}

pub fn render_template_as_prompt(t: &Template) -> String {
    let mut out = String::new();
    let desc = t.description.trim();
    if !desc.is_empty() {
        out.push_str(desc);
        out.push_str("\n\n");
    }
    out.push_str("Structure the report with these sections, in this order:\n\n");
    for (i, s) in t.sections.iter().enumerate() {
        let instruction = s.instruction.trim().trim_end_matches('.').trim_end();
        let mut line = format!("{}. **{}** — {}.", i + 1, s.title.trim(), instruction);
        match s.format.as_str() {
            "list" => line.push_str(" Use a bulleted list."),
            "string" => line.push_str(" Keep this to a single short line."),
            _ => {}
        }
        if let Some(f) = s.item_format.as_ref().or(s.example_item_format.as_ref()) {
            line.push_str(&format!(" Format each item as: {f}"));
        }
        out.push_str(&line);
        out.push('\n');
    }
    out.trim_end().to_string()
}

/// Template ids to convert: everything except the reserved `auto`, minus hidden ones that no
/// meeting still references.
fn select_templates(
    all: Vec<String>,
    hidden: &BTreeSet<String>,
    referenced: &BTreeSet<String>,
) -> Vec<String> {
    all.into_iter()
        .filter(|id| id != AUTO_ID && (!hidden.contains(id) || referenced.contains(id)))
        .collect()
}

pub async fn convert_templates_once(pool: &SqlitePool) -> Result<Option<ConversionReport>> {
    if SummaryPromptRepository::get_meta(pool, MARKER)
        .await?
        .is_some()
    {
        return Ok(None);
    }
    let mut report = ConversionReport::default();
    let referenced: BTreeSet<String> = sqlx::query_scalar(
        "SELECT DISTINCT TRIM(template_id) FROM meetings \
         WHERE template_id IS NOT NULL AND TRIM(template_id) != ''",
    )
    .fetch_all(pool)
    .await?
    .into_iter()
    .collect();
    let mut id_map: HashMap<String, String> = HashMap::new();

    // Auto preset first, so it exists (and becomes the default) before anything else.
    let auto_name = SummaryPromptRepository::unique_name(pool, AUTO_PRESET_NAME).await?;
    let auto =
        SummaryPromptRepository::create(pool, &auto_name, AUTO_PRESET_BODY, true, true).await?;
    SummaryPromptRepository::set_default(pool, &auto.id).await?;
    id_map.insert(AUTO_ID.into(), auto.id.clone());
    report.prompts_created += 1;

    for id in select_templates(list_template_ids(), &hidden_template_ids(), &referenced) {
        let t = match get_template(&id) {
            Ok(t) => t,
            Err(e) => {
                warn!("Skipping template '{id}' during prompt conversion: {e}");
                report.skipped.push(format!("{id}: {e}"));
                continue;
            }
        };
        let base = sanitize_prompt_name(&t.name).unwrap_or_else(|_| id.clone());
        let rendered = render_template_as_prompt(&t);
        let body = sanitize_prompt_body(&rendered)
            .unwrap_or_else(|_| rendered.chars().take(MAX_PROMPT_CHARS).collect());
        let created = async {
            let name = SummaryPromptRepository::unique_name(pool, &base).await?;
            SummaryPromptRepository::create(pool, &name, &body, true, true).await
        }
        .await;
        match created {
            Ok(p) => {
                id_map.insert(id, p.id);
                report.prompts_created += 1;
            }
            Err(e) => {
                warn!("Skipping template '{id}' during prompt conversion: {e:#}");
                report.skipped.push(format!("{id}: {e:#}"));
            }
        }
    }

    // Per-meeting mapping (the default prompt means "follow default/series": leave NULL).
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT id, template_id FROM meetings \
         WHERE template_id IS NOT NULL AND TRIM(template_id) != ''",
    )
    .fetch_all(pool)
    .await?;
    for (meeting_id, template_id) in rows {
        if let Some(prompt_id) = id_map.get(template_id.trim()) {
            if *prompt_id != auto.id {
                SummaryPromptRepository::set_meeting_prompt_id(pool, &meeting_id, Some(prompt_id))
                    .await?;
                report.meetings_mapped += 1;
            }
        }
    }
    // Marker last: a failure above leaves it unset so the next launch retries.
    SummaryPromptRepository::set_meta(pool, MARKER, "1").await?;
    Ok(Some(report))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::repositories::meeting::test_support::memory_db;
    use crate::database::repositories::meeting::MeetingsRepository;
    use crate::summary::templates::TemplateSection;
    use chrono::Utc;

    fn section(title: &str, instruction: &str, format: &str) -> TemplateSection {
        TemplateSection {
            title: title.into(),
            instruction: instruction.into(),
            format: format.into(),
            item_format: None,
            example_item_format: None,
        }
    }

    #[test]
    fn render_template_as_prompt_is_deterministic_prose() {
        let t = Template {
            name: "Standup".into(),
            description: "Daily sync".into(),
            sections: vec![
                section("Yesterday", "What was done", "list"),
                section("Blockers", "What is blocking", "paragraph"),
            ],
        };
        assert_eq!(
            render_template_as_prompt(&t),
            "Daily sync\n\nStructure the report with these sections, in this order:\n\n\
             1. **Yesterday** — What was done. Use a bulleted list.\n\
             2. **Blockers** — What is blocking."
        );
    }

    #[test]
    fn render_handles_string_item_format_and_empty_description() {
        let mut s = section("Owner", "Who owns it.", "string");
        s.item_format = Some("| a | b |".into());
        let t = Template {
            name: "X".into(),
            description: "  ".into(),
            sections: vec![s],
        };
        assert_eq!(
            render_template_as_prompt(&t),
            "Structure the report with these sections, in this order:\n\n\
             1. **Owner** — Who owns it. Keep this to a single short line. Format each item as: | a | b |"
        );
    }

    #[test]
    fn auto_preset_body_is_the_resolver_fallback() {
        assert_eq!(
            AUTO_PRESET_BODY,
            crate::summary::prompts_resolve::FALLBACK_PROMPT
        );
    }

    #[test]
    fn select_templates_skips_auto_and_unreferenced_hidden() {
        let all = vec!["auto", "a", "b", "c"]
            .into_iter()
            .map(String::from)
            .collect();
        let hidden: BTreeSet<String> = ["b", "c"].into_iter().map(String::from).collect();
        let referenced: BTreeSet<String> = ["c", "auto"].into_iter().map(String::from).collect();
        assert_eq!(select_templates(all, &hidden, &referenced), vec!["a", "c"]);
    }

    async fn builtin_names() -> Vec<String> {
        crate::summary::templates::list_template_ids()
            .into_iter()
            .filter(|id| crate::summary::templates::is_builtin_or_bundled(id))
            .filter_map(|id| get_template(&id).ok().map(|t| t.name))
            .collect()
    }

    #[tokio::test]
    async fn converts_builtins_and_auto_preset_and_sets_auto_default() {
        let pool = memory_db().await;
        let report = convert_templates_once(&pool).await.unwrap().unwrap();
        let prompts = SummaryPromptRepository::list(&pool).await.unwrap();
        assert_eq!(prompts.len(), report.prompts_created);
        let names: Vec<&str> = prompts.iter().map(|p| p.name.as_str()).collect();
        assert!(names.contains(&AUTO_PRESET_NAME), "{names:?}");
        let builtins = builtin_names().await;
        assert!(builtins.len() >= 5, "{builtins:?}");
        for b in &builtins {
            assert!(names.contains(&b.as_str()), "missing {b}: {names:?}");
        }
        assert!(prompts.iter().all(|p| p.extract_action_items));
        let def = SummaryPromptRepository::get_default(&pool)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(def.name, AUTO_PRESET_NAME);
        assert_eq!(def.body, AUTO_PRESET_BODY);
    }

    #[tokio::test]
    async fn second_run_is_a_noop_via_marker() {
        let pool = memory_db().await;
        convert_templates_once(&pool).await.unwrap().unwrap();
        let before = SummaryPromptRepository::list(&pool).await.unwrap().len();
        assert!(convert_templates_once(&pool).await.unwrap().is_none());
        assert_eq!(
            SummaryPromptRepository::list(&pool).await.unwrap().len(),
            before
        );
        assert!(SummaryPromptRepository::get_meta(&pool, MARKER)
            .await
            .unwrap()
            .is_some());
    }

    #[tokio::test]
    async fn meetings_map_to_converted_prompts_except_default() {
        let pool = memory_db().await;
        let mut ids = Vec::new();
        for tpl in [
            Some("standard_meeting"),
            Some("auto"),
            Some("nonexistent"),
            None,
        ] {
            let id = MeetingsRepository::create_meeting(
                &pool,
                Some("M".into()),
                None,
                None,
                None,
                Some(Utc::now()),
            )
            .await
            .unwrap();
            MeetingsRepository::set_meeting_template(&pool, &id, tpl)
                .await
                .unwrap();
            ids.push(id);
        }
        convert_templates_once(&pool).await.unwrap().unwrap();
        let std_prompt = SummaryPromptRepository::list(&pool)
            .await
            .unwrap()
            .into_iter()
            .find(|p| p.name == "Standard Meeting Notes")
            .unwrap();
        let mut got = Vec::new();
        for id in &ids {
            got.push(
                SummaryPromptRepository::meeting_fields(&pool, id)
                    .await
                    .unwrap()
                    .unwrap()
                    .summary_prompt_id,
            );
        }
        assert_eq!(got, vec![Some(std_prompt.id), None, None, None]);
    }

    #[tokio::test]
    async fn name_collisions_get_numeric_suffix() {
        let pool = memory_db().await;
        SummaryPromptRepository::create(&pool, "Standard Meeting Notes", "mine", true, true)
            .await
            .unwrap();
        convert_templates_once(&pool).await.unwrap().unwrap();
        let names: Vec<String> = SummaryPromptRepository::list(&pool)
            .await
            .unwrap()
            .into_iter()
            .map(|p| p.name)
            .collect();
        assert!(
            names.contains(&"Standard Meeting Notes 2".to_string()),
            "{names:?}"
        );
    }

    #[tokio::test]
    async fn default_exists_and_marker_set_after_success() {
        let pool = memory_db().await;
        convert_templates_once(&pool).await.unwrap().unwrap();
        assert!(SummaryPromptRepository::get_default(&pool)
            .await
            .unwrap()
            .is_some());
        assert_eq!(
            SummaryPromptRepository::get_meta(&pool, MARKER)
                .await
                .unwrap()
                .as_deref(),
            Some("1")
        );
    }
}
