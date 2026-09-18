use super::*;
use crate::summary::templates::Template;

// specs/0061 W6 regression: a meeting pinned to a template id that no
// longer resolves (a built-in removed in an app update, or a deleted
// custom override) must still get a real template from the exact
// function `process_transcript_background`'s fixed-template match arm
// calls — never the old `update_process_failed` path. Calling
// `SummaryService::resolve_fixed_template` directly exercises that real
// production code, not a reimplementation of it.
#[test]
fn resolve_fixed_template_falls_back_to_default_when_id_does_not_resolve() {
    let template =
        SummaryService::resolve_fixed_template("meeting-under-test", "psychatric_session");
    let default_template = templates::get_template(templates::DEFAULT_TEMPLATE_ID)
        .expect("the default template must resolve");
    assert_eq!(template.name, default_template.name);
    assert_eq!(template.sections.len(), default_template.sections.len());
}

#[test]
fn resolve_fixed_template_uses_the_real_template_when_it_resolves() {
    let template = SummaryService::resolve_fixed_template("meeting-under-test", "daily_standup");
    assert_eq!(template.name, "Daily Standup");
}

fn sample_cache_source() -> SummaryCacheSource {
    let template_fingerprint = stable_text_fingerprint("standard template prompt");
    build_summary_cache_source(
        "transcript body",
        "custom prompt",
        "standard_meeting",
        &template_fingerprint,
        3700,
        "ollama",
        "gemma3:1b",
        Some("http://localhost:11434"),
        None,
        None,
        None,
        None,
    )
}

fn test_template(section_title: &str) -> Template {
    Template {
        name: "Test".to_string(),
        description: "Test template".to_string(),
        sections: vec![crate::summary::templates::TemplateSection {
            title: section_title.to_string(),
            instruction: "Summarize this section".to_string(),
            format: "paragraph".to_string(),
            item_format: None,
            example_item_format: None,
        }],
    }
}

#[test]
fn test_template_cache_fingerprint_changes_with_rendered_template() {
    assert_ne!(
        template_cache_fingerprint(&test_template("Summary")),
        template_cache_fingerprint(&test_template("Decisions"))
    );
}

#[test]
fn test_legacy_english_markdown_field_is_cache_miss() {
    let raw = serde_json::json!({
        "markdown": "translated",
        "english_markdown": "# Old English\nBody"
    })
    .to_string();

    assert_eq!(
        extract_cached_english_markdown(&raw, &sample_cache_source(), Some("de")).unwrap(),
        None
    );
}

#[test]
fn test_matching_source_changed_translation_target_reuses_cache() {
    let source = sample_cache_source();
    let raw = build_summary_result_json(
        "# Reunion\n## Points\nBonjour",
        "# Meeting\n## Points\nHello",
        source.clone(),
        Some("fr"),
    )
    .to_string();

    assert_eq!(
        extract_cached_english_markdown(&raw, &source, Some("de")).unwrap(),
        Some("# Meeting\n## Points\nHello".to_string())
    );
}

#[test]
fn test_same_language_regeneration_rejects_cache() {
    let source = sample_cache_source();
    let raw = build_summary_result_json(
        "# Reunion\n## Points\nBonjour",
        "# Meeting\n## Points\nHello",
        source.clone(),
        Some("fr"),
    )
    .to_string();

    assert_eq!(
        extract_cached_english_markdown(&raw, &source, Some("fr")).unwrap(),
        None
    );
}

#[test]
fn test_changed_summary_inputs_reject_cache() {
    let source = sample_cache_source();
    let template_fingerprint = source.template_fingerprint.clone();
    let raw = build_summary_result_json(
        "# Reunion\n## Points\nBonjour",
        "# Meeting\n## Points\nHello",
        source,
        Some("fr"),
    )
    .to_string();

    let changed_sources = [
        build_summary_cache_source(
            "changed transcript",
            "custom prompt",
            "standard_meeting",
            &template_fingerprint,
            3700,
            "ollama",
            "gemma3:1b",
            Some("http://localhost:11434"),
            None,
            None,
            None,
            None,
        ),
        build_summary_cache_source(
            "transcript body",
            "changed prompt",
            "standard_meeting",
            &template_fingerprint,
            3700,
            "ollama",
            "gemma3:1b",
            Some("http://localhost:11434"),
            None,
            None,
            None,
            None,
        ),
        build_summary_cache_source(
            "transcript body",
            "custom prompt",
            "daily_standup",
            &template_fingerprint,
            3700,
            "ollama",
            "gemma3:1b",
            Some("http://localhost:11434"),
            None,
            None,
            None,
            None,
        ),
        build_summary_cache_source(
            "transcript body",
            "custom prompt",
            "standard_meeting",
            &template_fingerprint,
            3700,
            "openai",
            "gemma3:1b",
            Some("http://localhost:11434"),
            None,
            None,
            None,
            None,
        ),
        build_summary_cache_source(
            "transcript body",
            "custom prompt",
            "standard_meeting",
            &template_fingerprint,
            3700,
            "ollama",
            "qwen2.5:3b",
            Some("http://localhost:11434"),
            None,
            None,
            None,
            None,
        ),
        build_summary_cache_source(
            "transcript body",
            "custom prompt",
            "standard_meeting",
            &template_fingerprint,
            3700,
            "ollama",
            "gemma3:1b",
            Some("http://localhost:11500"),
            None,
            None,
            None,
            None,
        ),
        build_summary_cache_source(
            "transcript body",
            "custom prompt",
            "standard_meeting",
            &template_fingerprint,
            3700,
            "ollama",
            "gemma3:1b",
            Some("http://localhost:11434"),
            Some("https://custom.example/v1"),
            Some(2048),
            Some(0.2),
            Some(0.9),
        ),
    ];

    for changed_source in changed_sources {
        assert_eq!(
            extract_cached_english_markdown(&raw, &changed_source, Some("de")).unwrap(),
            None
        );
    }
}

#[test]
fn test_changed_template_content_rejects_cache() {
    let source = sample_cache_source();
    let raw = build_summary_result_json(
        "# Reunion\n## Points\nBonjour",
        "# Meeting\n## Points\nHello",
        source.clone(),
        Some("fr"),
    )
    .to_string();

    let changed_template = SummaryCacheSource {
        template_fingerprint: stable_text_fingerprint("changed template prompt"),
        ..source
    };

    assert_eq!(
        extract_cached_english_markdown(&raw, &changed_template, Some("de")).unwrap(),
        None
    );
}

#[test]
fn test_changed_token_threshold_rejects_cache() {
    let source = sample_cache_source();
    let raw = build_summary_result_json(
        "# Reunion\n## Points\nBonjour",
        "# Meeting\n## Points\nHello",
        source.clone(),
        Some("fr"),
    )
    .to_string();

    let changed_threshold = SummaryCacheSource {
        token_threshold: 8192,
        ..source
    };

    assert_eq!(
        extract_cached_english_markdown(&raw, &changed_threshold, Some("de")).unwrap(),
        None
    );
}

#[test]
fn test_result_json_stores_stripped_display_markdown_but_keeps_cache_title() {
    // The completion arm strips the title ONCE and hands the stripped string to the
    // builder (which stores it verbatim); the English cache keeps its title.
    let result = build_summary_result_json(
        &strip_title_if_present("# Translated Title\n## Decisions\nDone"),
        "# English Title\n## Decisions\nDone",
        sample_cache_source(),
        Some("fr"),
    );

    assert_eq!(result["markdown"], "## Decisions\nDone");
    assert_eq!(
        result["english_cache"]["markdown"],
        "# English Title\n## Decisions\nDone"
    );
}

/// Regression (specs/0034 acceptance #4, cross-path fingerprint): the background
/// extraction spawn fingerprints the SAME string the completion arm stores at
/// `$.markdown` — which is exactly what `api_extract_action_items` reads back for a
/// manual "Scan again". If the two paths ever diverge again (e.g. one fingerprints
/// the raw titled markdown), an auto-then-manual run on an UNCHANGED summary stops
/// being a ledger no-op and every scan becomes a full re-extraction.
#[test]
fn auto_then_manual_extraction_fingerprints_match_on_unchanged_summary() {
    use crate::action_items::diff::extraction_fingerprint;

    let final_markdown = "# Weekly Sync\n## Action Items\n- send the deck to Alice";
    let user_notes = Some("deck first");

    // The completion arm: strip once, store, and fingerprint the same string.
    let auto_extraction_input = strip_title_if_present(final_markdown);
    let stored = build_summary_result_json(
        &auto_extraction_input,
        final_markdown,
        sample_cache_source(),
        None,
    )
    .to_string();

    // The manual command: read `$.markdown` back from the stored result row.
    let manual_extraction_input: String = serde_json::from_str::<serde_json::Value>(&stored)
        .unwrap()["markdown"]
        .as_str()
        .unwrap()
        .to_string();

    assert_eq!(
        extraction_fingerprint(&auto_extraction_input, user_notes),
        extraction_fingerprint(&manual_extraction_input, user_notes),
        "auto and manual extraction must fingerprint the identical stored representation"
    );
}

#[test]
fn test_extract_cached_english_from_malformed_json_errors() {
    let raw = r#"{ not valid json"#;
    assert!(extract_cached_english_markdown(raw, &sample_cache_source(), Some("de")).is_err());
}

/// Shape lock for full-text search (specs/0033): the FTS migration
/// (`migrations/20260706000000_add_fts5_search.sql`) indexes summaries via a
/// generated column computed as
/// `CASE WHEN result IS NOT NULL AND json_valid(result) THEN json_extract(result, '$.markdown') END`,
/// where `result` is the JSON THIS builder produces. If the result shape
/// ever moves the display markdown off `$.markdown`, that column silently
/// becomes NULL and summaries drop out of search with no other test failing
/// — this test fails first. Keep the SQL expression here byte-identical to
/// the migration's `summary_text` definition.
#[tokio::test]
async fn summary_result_json_markdown_is_extractable_by_fts_migration() {
    use sqlx::{ConnectOptions, Row};

    let markdown = "## Decisions\nadopt the roadmap";
    // No leading H1, so the stored `$.markdown` is byte-identical to the input.
    let result =
        build_summary_result_json(markdown, markdown, sample_cache_source(), None).to_string();

    let mut conn = sqlx::sqlite::SqliteConnectOptions::new()
        .in_memory(true)
        .connect()
        .await
        .expect("in-memory SQLite connection");

    let extracted: Option<String> = sqlx::query(
        "SELECT CASE WHEN ? IS NOT NULL AND json_valid(?) \
                THEN json_extract(?, '$.markdown') END",
    )
    .bind(&result)
    .bind(&result)
    .bind(&result)
    .fetch_one(&mut conn)
    .await
    .expect("json_extract over the builder's output")
    .get(0);

    assert_eq!(
        extracted.as_deref(),
        Some(markdown),
        "summary display markdown must live at $.markdown in summary_processes.result \
         (the FTS migration's generated column reads it there)"
    );
}
