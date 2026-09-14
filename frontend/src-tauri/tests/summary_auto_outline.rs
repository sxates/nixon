//! specs/0053 W3: Auto resolves its structure mid-pipeline, and a fixed
//! template still takes the pre-0053 path untouched.

use app_lib::summary::cache_key::template_cache_fingerprint;
use app_lib::summary::outline::{
    fallback_outline, to_template, validate, Outline, OutlineSection, SectionRole,
    AUTO_TEMPLATE_ID,
};
use app_lib::summary::templates;

/// Auto is a reserved id, not a template file. Asking the loader for it must
/// fail — nothing should ever load "auto" from disk.
#[test]
fn auto_is_not_a_loadable_template_file() {
    assert!(
        templates::get_template(AUTO_TEMPLATE_ID).is_err(),
        "'auto' must be reserved, never loaded as a file"
    );
}

/// A derived outline must satisfy Template's own validation, or the final
/// synthesis prompt would be malformed.
#[test]
fn a_derived_outline_produces_a_valid_template() {
    let outline = Outline {
        has_commitments: true,
        derived: true,
        sections: vec![
            OutlineSection {
                title: "Where We Landed".to_string(),
                instruction: "Summarize the outcome.".to_string(),
                format: "paragraph".to_string(),
                item_format: None,
                role: Some(SectionRole::Overview),
            },
            OutlineSection {
                title: "Open Disagreements".to_string(),
                instruction: "List unresolved points.".to_string(),
                format: "list".to_string(),
                item_format: None,
                role: None,
            },
            OutlineSection {
                title: "Follow-ups".to_string(),
                instruction: "List who committed to what.".to_string(),
                format: "list".to_string(),
                item_format: None,
                role: Some(SectionRole::Commitments),
            },
        ],
    };
    assert_eq!(validate(&outline), Ok(()));

    let template = to_template(&outline);
    assert!(template.validate().is_ok());

    let structure = template.to_markdown_structure();
    assert!(structure.contains("Where We Landed"));
    assert!(structure.contains("Follow-ups"));

    let instructions = template.to_section_instructions();
    assert!(instructions.contains("Summarize the outcome"));
}

/// The fallback must render just like standard_meeting, so a derivation
/// failure degrades to exactly today's default output shape.
#[test]
fn the_fallback_renders_like_standard_meeting() {
    let fallback = to_template(&fallback_outline());
    let standard = templates::get_template("standard_meeting").expect("built-in must load");

    let fallback_titles: Vec<_> = fallback.sections.iter().map(|s| &s.title).collect();
    let standard_titles: Vec<_> = standard.sections.iter().map(|s| &s.title).collect();
    assert_eq!(fallback_titles, standard_titles);
}

/// Auto must never emit the forced transcript-reference table that
/// standard_meeting mandates — that shape is a main reason for this work.
#[test]
fn auto_does_not_force_the_transcript_reference_table() {
    let auto = to_template(&fallback_outline());
    for section in &auto.sections {
        assert!(
            section.item_format.is_none(),
            "Auto must not force an item_format on '{}'",
            section.title
        );
    }

    let standard = templates::get_template("standard_meeting").unwrap();
    assert!(
        standard.sections.iter().any(|s| s.item_format.is_some()),
        "guard: standard_meeting is the template that forces a table"
    );
}

/// A persisted outline must fingerprint stably, or every regeneration would
/// miss the cache and re-run the whole pipeline.
#[test]
fn the_same_outline_fingerprints_identically() {
    let template = to_template(&fallback_outline());
    assert_eq!(
        template_cache_fingerprint(&template),
        template_cache_fingerprint(&to_template(&fallback_outline()))
    );
}

/// A different outline must fingerprint differently, or a re-derived structure
/// would silently serve the old summary.
#[test]
fn a_different_outline_fingerprints_differently() {
    let mut changed = fallback_outline();
    changed.sections[1].title = "Something Else Entirely".to_string();
    assert_ne!(
        template_cache_fingerprint(&to_template(&fallback_outline())),
        template_cache_fingerprint(&to_template(&changed))
    );
}
