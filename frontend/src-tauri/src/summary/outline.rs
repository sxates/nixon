//! Auto summary structure (specs/0053 W3): derive the section list from the
//! meeting's own content instead of a fixed template.
//!
//! Runs AFTER the map/reduce collapse in `processor::generate_meeting_summary`,
//! so on a long meeting it reads the already-reduced text rather than the
//! transcript — the cheapest call in the pipeline.
//!
//! **Not grammar-constrained.** The brief for this task called for a GBNF
//! grammar (`OUTLINE_GBNF`) via `GenerationOptions::deterministic_json(grammar)`.
//! That was deliberately dropped: grammar-constrained decoding on the pinned
//! `llama-cpp-2 =0.1.146` aborts the whole sidecar process (`SIGABRT`,
//! `llama-grammar.cpp:940: GGML_ASSERT(!stacks.empty()) failed`) for every
//! grammar tested, including llama.cpp's own reference `json.gbnf` — see
//! `summary_engine::options::GenerationOptions::grammar`'s doc comment. The
//! action-item extractor already had its grammar wiring removed for the same
//! reason. `GenerationOptions::deterministic_json()` (no arguments) is used
//! instead: greedy decoding, neutral repetition penalties, and `json_mode`
//! (which drives `response_format: {"type":"json_object"}` on Ollama/OpenAI-
//! compatible providers — that part does work).
//!
//! With no grammar, structural validity is enforced entirely on the Rust side:
//! [`validate`] plus the `<...>`/`[...]` template-echo title guard, and
//! [`fallback_outline`] as the last resort. [`derive_outline`] is infallible by
//! contract — transport error, unparseable reply (including a chatty model that
//! wraps its JSON in prose), or failed validation all log a warning and fall
//! back; a summary must never fail because Auto could not pick a shape.
//!
//! Everything downstream is untouched: the derived `Template` feeds the same
//! `to_markdown_structure` / `to_section_instructions` path a fixed template
//! does, so chunking, caching, notes grounding, speaker attribution and role
//! weighting all behave identically.

use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use crate::summary::llm_client::LLMProvider;
use crate::summary::summary_engine::options::GenerationOptions;
use crate::summary::templates::{Template, TemplateSection};

/// The reserved template id for content-derived structure. Never a file on disk.
pub const AUTO_TEMPLATE_ID: &str = "auto";

/// Fallback when derivation fails at any stage.
pub const FALLBACK_TEMPLATE_ID: &str = "standard_meeting";

const MIN_SECTIONS: usize = 3;
const MAX_SECTIONS: usize = 6;

/// Where a summary's structure comes from for this run.
#[derive(Debug)]
pub enum TemplateChoice<'a> {
    /// A fixed template, resolved before generation as it always was.
    Fixed(&'a Template),
    /// Derive from content mid-pipeline (`template_id == "auto"` and no
    /// outline is persisted yet).
    DeriveAuto,
}

/// A section's structural role. Validation-only — dropped by [`to_template`],
/// so the rendered prompt is shaped exactly like a fixed template's. Exists so
/// the commitments section is found structurally rather than by title-matching:
/// the model is free to call it "Next Steps", "Follow-ups", or "Who's doing
/// what", which is the entire point of Auto.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SectionRole {
    Overview,
    Commitments,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OutlineSection {
    pub title: String,
    pub instruction: String,
    pub format: String,
    #[serde(default)]
    pub item_format: Option<String>,
    #[serde(default)]
    pub role: Option<SectionRole>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Outline {
    pub sections: Vec<OutlineSection>,
    pub has_commitments: bool,
    /// True only when this came from a successful model derivation, never
    /// from [`fallback_outline`]. Deliberately **not serialized** — it's a
    /// same-run signal, irrelevant once persisted (a stored outline is by
    /// definition one worth keeping). `processor::generate_meeting_summary`
    /// reads it to decide whether to hand the outline back to the caller for
    /// persistence at all: a transient failure (transport error, unparseable
    /// reply, failed validation) must not get written to
    /// `SummaryOutlineRepository` and permanently pin the meeting to the
    /// fallback shape — the next run should retry derivation from scratch
    /// (specs/0053 I2).
    #[serde(skip)]
    pub derived: bool,
}

const OUTLINE_SYSTEM_PROMPT: &str = "You design the structure of a meeting report. You will be given a condensed account of one meeting. Decide the sections that best fit THIS meeting's content, then return them as JSON.\n\nRules:\n- Return between 3 and 6 sections.\n- The FIRST section must be a prose overview: format \"paragraph\", role \"overview\".\n- If anyone committed to do anything, or work was assigned or agreed, set \"has_commitments\" to true and include exactly one section with role \"commitments\". Title it however suits this meeting (\"Next Steps\", \"Follow-ups\", \"Who's doing what\").\n- If nobody committed to anything, set \"has_commitments\" to false and omit the commitments section.\n- Every other section has role null.\n- \"instruction\" tells the writer what to put in that section, in one sentence.\n- \"format\" is \"paragraph\" for prose, \"list\" for bullets, \"string\" for a single short value.\n- \"item_format\" is almost always null. Give a markdown table header ONLY when the content is genuinely tabular.\n- Fit the sections to what actually happened. Do not pad with sections this meeting has no content for.\n- Titles are plain descriptive words. Never use angle brackets or square brackets.\n- Return ONLY the JSON object: keys \"has_commitments\" (bool) and \"sections\" (array of objects with \"title\", \"instruction\", \"format\", \"item_format\", \"role\"). No prose, no markdown fence.";

/// Reject an outline the pipeline cannot safely use.
///
/// **Privacy contract:** the returned `Err` string is a content-free
/// discriminant — section index and rule name only. Section titles,
/// instructions, and format values are model output derived from the
/// meeting's content and must never be interpolated into it, because this
/// string is logged at `warn` by `outline_from_reply` and this product's
/// premise is that meeting content never leaves the machine, including into
/// `~/Library/Logs` (specs/0053 C2).
pub fn validate(outline: &Outline) -> Result<(), String> {
    // Checked before the section-count bounds below: a meeting with commitments
    // that dropped its commitments section is a distinct, more specific failure
    // than "wrong section count" even when removing that section also happens to
    // put the total out of range (e.g. a 3-section outline losing its only
    // commitments section reads as "missing commitments", not "only 2 sections").
    let commitment_sections = outline
        .sections
        .iter()
        .filter(|s| s.role == Some(SectionRole::Commitments))
        .count();
    if outline.has_commitments && commitment_sections != 1 {
        // Load-bearing: action_items::extractor reads the SUMMARY as its
        // primary source. No commitments section => action items break silently.
        return Err(format!(
            "meeting has commitments so exactly one commitments section is required, found {commitment_sections}"
        ));
    }
    if !outline.has_commitments && commitment_sections > 0 {
        return Err("outline claims no commitments but declares a commitments section".to_string());
    }

    if outline.sections.len() < MIN_SECTIONS {
        return Err(format!(
            "outline has {} sections, minimum is {MIN_SECTIONS}",
            outline.sections.len()
        ));
    }
    if outline.sections.len() > MAX_SECTIONS {
        return Err(format!(
            "outline has {} sections, maximum is {MAX_SECTIONS}",
            outline.sections.len()
        ));
    }

    let first = &outline.sections[0];
    if first.role != Some(SectionRole::Overview) || first.format != "paragraph" {
        return Err(
            "the first section must be a prose overview (role \"overview\", format \"paragraph\")"
                .to_string(),
        );
    }

    for (i, section) in outline.sections.iter().enumerate() {
        if section.title.trim().is_empty() {
            return Err(format!("section {i} has an empty title"));
        }
        if section.instruction.trim().is_empty() {
            // Content-free discriminant: `section.title` is model output
            // derived from the meeting and must never reach the log (specs/0053
            // C2) — index + rule name is all downstream diagnostics need.
            return Err(format!("section {i}: empty instruction"));
        }
        if !matches!(section.format.as_str(), "paragraph" | "list" | "string") {
            // Neither the section title NOR the (also model-authored) format
            // value is safe to log — e.g. a 4B model emitting `format: "table"`.
            return Err(format!(
                "section {i}: format not in {{paragraph, list, string}}"
            ));
        }
        if has_template_echo(&section.title) {
            return Err(format!(
                "section {i}: title looks like an unfilled template token"
            ));
        }
    }

    Ok(())
}

/// Mirrors the `<...>` / `[...]` guard `processor::is_valid_meeting_title` uses:
/// a model that parrots the scaffold must not get it persisted as a title.
fn has_template_echo(title: &str) -> bool {
    let t = title.trim();
    (t.contains('<') && t.contains('>')) || (t.contains('[') && t.contains(']'))
}

/// Convert to the `Template` the rest of the pipeline consumes. `role` is
/// dropped here — it never reaches a prompt.
pub fn to_template(outline: &Outline) -> Template {
    Template {
        name: "Auto".to_string(),
        description: "Structure derived from this meeting's content".to_string(),
        sections: outline
            .sections
            .iter()
            .map(|s| TemplateSection {
                title: s.title.clone(),
                instruction: s.instruction.clone(),
                format: s.format.clone(),
                item_format: s.item_format.clone(),
                example_item_format: None,
            })
            .collect(),
    }
}

/// The shape used when derivation fails: `standard_meeting`'s sections, so a
/// failure degrades to exactly today's default rather than to nothing.
pub fn fallback_outline() -> Outline {
    Outline {
        has_commitments: true,
        derived: false,
        sections: vec![
            OutlineSection {
                title: "Summary".to_string(),
                instruction:
                    "Provide a brief, one-paragraph executive summary of the entire meeting"
                        .to_string(),
                format: "paragraph".to_string(),
                item_format: None,
                role: Some(SectionRole::Overview),
            },
            OutlineSection {
                title: "Key Decisions".to_string(),
                instruction: "List the most important decisions made during the meeting"
                    .to_string(),
                format: "list".to_string(),
                item_format: None,
                role: None,
            },
            OutlineSection {
                title: "Action Items".to_string(),
                instruction: "List all assigned tasks with their owners and due date".to_string(),
                format: "list".to_string(),
                item_format: None,
                role: Some(SectionRole::Commitments),
            },
            OutlineSection {
                title: "Discussion Highlights".to_string(),
                instruction:
                    "Summarize the main topics of discussion, key arguments, and important insights"
                        .to_string(),
                format: "paragraph".to_string(),
                item_format: None,
                role: None,
            },
        ],
    }
}

/// Parse and validate a model's raw reply into an outline, falling back to
/// [`fallback_outline`] on any failure. Pure and synchronous (no network call)
/// so every failure path — empty reply, prose with no JSON, JSON of the wrong
/// shape, an out-of-range section count — is unit-testable without a live LLM.
///
/// Privacy: only lengths/counts/error kinds are logged, never model content.
fn outline_from_reply(reply: &str) -> Outline {
    let cleaned = crate::summary::processor::clean_llm_markdown_output(reply);
    let sliced = match (cleaned.find('{'), cleaned.rfind('}')) {
        (Some(start), Some(end)) if start < end => &cleaned[start..=end],
        _ => {
            warn!(
                "Auto outline reply had no JSON object ({} chars); falling back to {FALLBACK_TEMPLATE_ID}",
                reply.len()
            );
            return fallback_outline();
        }
    };

    let outline: Outline = match serde_json::from_str(sliced) {
        Ok(outline) => outline,
        Err(e) => {
            // Privacy: never log `e`'s Display. For `invalid type` /
            // `unknown variant` / `unknown field`, serde_json's Display
            // embeds the offending (model-authored) value verbatim. Category
            // + position is enough to debug a shape mismatch without it
            // (specs/0053 C2).
            warn!(
                "Auto outline reply did not deserialize (category={:?}, line={}, column={}); falling back to {FALLBACK_TEMPLATE_ID}",
                e.classify(),
                e.line(),
                e.column()
            );
            return fallback_outline();
        }
    };

    if let Err(e) = validate(&outline) {
        // `e` is a content-free discriminant (section index + rule name) by
        // construction — see `validate`'s doc.
        warn!("Auto outline failed validation ({e}); falling back to {FALLBACK_TEMPLATE_ID}");
        return fallback_outline();
    }

    info!(
        "Auto outline derived: {} sections, has_commitments={}",
        outline.sections.len(),
        outline.has_commitments
    );
    Outline {
        derived: true,
        ..outline
    }
}

/// Derive an outline from the reduced meeting content.
///
/// **Infallible by contract.** Any failure — transport, parse, or validation —
/// logs a warning (never model content, only lengths/counts/error kinds) and
/// returns [`fallback_outline`]. Auto must never block or fail a summary.
#[allow(clippy::too_many_arguments)]
pub async fn derive_outline(
    client: &reqwest::Client,
    provider: &LLMProvider,
    model_name: &str,
    api_key: &str,
    content: &str,
    ollama_endpoint: Option<&str>,
    custom_openai_endpoint: Option<&str>,
    app_data_dir: Option<&std::path::PathBuf>,
    cancellation_token: Option<&CancellationToken>,
) -> Outline {
    let user_prompt = format!(
        "Design the report structure for this meeting.\n\n<meeting>\n{content}\n</meeting>\n\nReturn ONLY the JSON object."
    );

    let reply = match crate::summary::llm_client::generate_summary_with_options(
        client,
        provider,
        model_name,
        api_key,
        OUTLINE_SYSTEM_PROMPT,
        &user_prompt,
        ollama_endpoint,
        custom_openai_endpoint,
        None,
        Some(0.0),
        None,
        app_data_dir,
        cancellation_token,
        &GenerationOptions::deterministic_json(),
    )
    .await
    {
        Ok(reply) => reply,
        Err(e) => {
            warn!("Auto outline call failed ({e}); falling back to {FALLBACK_TEMPLATE_ID}");
            return fallback_outline();
        }
    };

    outline_from_reply(&reply)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn section(title: &str, format: &str, role: Option<SectionRole>) -> OutlineSection {
        OutlineSection {
            title: title.to_string(),
            instruction: format!("Write the {title} section."),
            format: format.to_string(),
            item_format: None,
            role,
        }
    }

    fn valid_outline() -> Outline {
        Outline {
            has_commitments: true,
            derived: false,
            sections: vec![
                section("Overview", "paragraph", Some(SectionRole::Overview)),
                section("What We Decided", "list", None),
                section("Who's Doing What", "list", Some(SectionRole::Commitments)),
            ],
        }
    }

    #[test]
    fn a_well_formed_outline_validates() {
        assert_eq!(validate(&valid_outline()), Ok(()));
    }

    #[test]
    fn fewer_than_three_sections_is_rejected() {
        let mut o = valid_outline();
        o.sections.truncate(2);
        assert!(validate(&o).is_err());
    }

    #[test]
    fn more_than_six_sections_is_rejected() {
        let mut o = valid_outline();
        while o.sections.len() <= 6 {
            o.sections.push(section("Filler", "list", None));
        }
        assert!(validate(&o).is_err());
    }

    #[test]
    fn the_first_section_must_be_a_prose_overview() {
        let mut o = valid_outline();
        o.sections[0].format = "list".to_string();
        assert!(validate(&o).is_err());

        let mut o = valid_outline();
        o.sections[0].role = None;
        assert!(validate(&o).is_err());
    }

    /// The load-bearing one: action_items::extractor reads the SUMMARY as its
    /// primary source, so an outline that drops the commitments section would
    /// silently break action items.
    #[test]
    fn commitments_section_is_required_when_the_meeting_has_commitments() {
        let mut o = valid_outline();
        o.sections
            .retain(|s| s.role != Some(SectionRole::Commitments));
        let err = validate(&o).expect_err("must reject");
        assert!(err.contains("commitments"), "unexpected error: {err}");
    }

    /// ...and is correctly NOT required when there are none.
    #[test]
    fn commitments_section_is_optional_when_there_are_no_commitments() {
        let mut o = valid_outline();
        o.has_commitments = false;
        o.sections
            .retain(|s| s.role != Some(SectionRole::Commitments));
        o.sections.push(section("Open Questions", "list", None));
        assert_eq!(validate(&o), Ok(()));
    }

    /// Auto's whole point: the commitments section is found by role, not by
    /// title, so the model can call it whatever suits the meeting.
    #[test]
    fn commitments_section_is_identified_by_role_not_title() {
        let mut o = valid_outline();
        o.sections[2].title = "Follow-ups from this call".to_string();
        assert_eq!(validate(&o), Ok(()));
    }

    #[test]
    fn unknown_formats_are_rejected() {
        let mut o = valid_outline();
        o.sections[1].format = "table".to_string();
        assert!(validate(&o).is_err());
    }

    /// Reuses processor.rs's template-echo guard: a model that parrots the
    /// scaffold must not have it persisted as a section title.
    #[test]
    fn template_echo_titles_are_rejected() {
        for bad in ["<Add title here>", "[AI-Generated Title]"] {
            let mut o = valid_outline();
            o.sections[1].title = bad.to_string();
            assert!(validate(&o).is_err(), "{bad} should be rejected");
        }
    }

    #[test]
    fn empty_titles_and_instructions_are_rejected() {
        let mut o = valid_outline();
        o.sections[1].title = "   ".to_string();
        assert!(validate(&o).is_err());

        let mut o = valid_outline();
        o.sections[1].instruction = String::new();
        assert!(validate(&o).is_err());
    }

    /// `role` is validation-only: the rendered prompt must be shaped exactly
    /// like a fixed template's.
    #[test]
    fn to_template_drops_role_and_preserves_everything_else() {
        let template = to_template(&valid_outline());
        assert_eq!(template.sections.len(), 3);
        assert_eq!(template.sections[0].title, "Overview");
        assert_eq!(template.sections[2].title, "Who's Doing What");
        assert_eq!(template.sections[1].format, "list");
        assert!(
            template.validate().is_ok(),
            "must satisfy Template's own rules"
        );
    }

    /// An outline the model emits must survive a JSON round-trip into the
    /// struct the pipeline consumes.
    #[test]
    fn model_shaped_json_deserializes() {
        let json = r#"{
            "has_commitments": true,
            "sections": [
                {"title":"Overview","instruction":"Summarize.","format":"paragraph","item_format":null,"role":"overview"},
                {"title":"Risks","instruction":"List risks.","format":"list","item_format":null,"role":null},
                {"title":"Next Steps","instruction":"List commitments.","format":"list","item_format":null,"role":"commitments"}
            ]
        }"#;
        let outline: Outline = serde_json::from_str(json).expect("must deserialize");
        assert_eq!(validate(&outline), Ok(()));
    }

    /// The fallback contract: garbage in, standard_meeting out — never an error
    /// that blocks a summary.
    #[test]
    fn fallback_outline_is_valid_and_matches_standard_meeting() {
        let fallback = fallback_outline();
        assert_eq!(validate(&fallback), Ok(()));
        assert!(to_template(&fallback).validate().is_ok());
    }

    // --- Self-review: every unexpected-model-output shape must resolve to
    // fallback_outline() through the pure (non-network) parsing path, with no
    // panic. These exercise `outline_from_reply` directly rather than
    // `derive_outline`, since the latter's only additional step is the network
    // call itself.

    #[test]
    fn empty_reply_falls_back() {
        assert_eq!(outline_from_reply(""), fallback_outline());
    }

    #[test]
    fn prose_with_no_json_falls_back() {
        assert_eq!(
            outline_from_reply("I'm sorry, I can't help with that request."),
            fallback_outline()
        );
    }

    /// Valid JSON, but not the Outline shape at all (wrong keys entirely) — must
    /// fail to deserialize and fall back rather than panicking on a missing field.
    #[test]
    fn valid_json_of_the_wrong_shape_falls_back() {
        assert_eq!(
            outline_from_reply(r#"{"foo": "bar", "baz": [1, 2, 3]}"#),
            fallback_outline()
        );
    }

    /// Valid JSON that deserializes cleanly into `Outline` but fails structural
    /// validation (here: only two sections) must also fall back.
    #[test]
    fn json_that_deserializes_but_fails_validation_falls_back() {
        let json = r#"{
            "has_commitments": false,
            "sections": [
                {"title":"Overview","instruction":"Summarize.","format":"paragraph","item_format":null,"role":"overview"},
                {"title":"Notes","instruction":"List notes.","format":"list","item_format":null,"role":null}
            ]
        }"#;
        assert_eq!(outline_from_reply(json), fallback_outline());
    }

    /// A model that ignores the "3 to 6 sections" instruction entirely (e.g.
    /// 40 sections) must fall back rather than persisting an unusable template.
    #[test]
    fn forty_sections_falls_back() {
        let mut sections = vec![serde_json::json!({
            "title": "Overview",
            "instruction": "Summarize.",
            "format": "paragraph",
            "item_format": null,
            "role": "overview"
        })];
        for i in 0..39 {
            sections.push(serde_json::json!({
                "title": format!("Section {i}"),
                "instruction": "Say something.",
                "format": "list",
                "item_format": null,
                "role": null
            }));
        }
        let payload = serde_json::json!({
            "has_commitments": false,
            "sections": sections
        })
        .to_string();

        assert_eq!(outline_from_reply(&payload), fallback_outline());
    }

    /// A chatty model that wraps its JSON in prose/markdown must still be
    /// recovered by the defensive `{`...`}` slice — this is the real-world case
    /// the (removed) grammar constraint would otherwise have prevented.
    #[test]
    fn json_wrapped_in_prose_is_recovered() {
        let reply = format!(
            "Sure! Here's the structure I'd suggest:\n\n```json\n{}\n```\n\nLet me know if you'd like changes.",
            serde_json::to_string(&valid_outline()).expect("outline serializes")
        );
        // A genuine successful derivation, unlike `valid_outline()` itself
        // (a plain test fixture) or a fallback, is marked `derived: true`
        // (specs/0053 I2) so the caller knows it is safe to persist.
        assert_eq!(
            outline_from_reply(&reply),
            Outline {
                derived: true,
                ..valid_outline()
            }
        );
    }

    /// specs/0053 I2: only a genuinely successful derivation is safe to
    /// persist — `fallback_outline()` itself, and every failure path that
    /// returns it, must carry `derived: false`.
    #[test]
    fn fallback_outline_is_never_marked_derived() {
        assert!(!fallback_outline().derived);
        assert!(!outline_from_reply("").derived);
        assert!(!outline_from_reply("not json at all").derived);
    }

    /// specs/0053 C2: a validation failure must never leak model-authored
    /// section content (title, instruction, or format) into the error
    /// string that gets logged.
    #[test]
    fn validation_errors_never_contain_section_title() {
        let secret_title = "Acme-Beta merger next steps";

        let mut empty_instruction = valid_outline();
        empty_instruction.sections[1].title = secret_title.to_string();
        empty_instruction.sections[1].instruction = String::new();
        let err = validate(&empty_instruction).expect_err("must reject");
        assert!(!err.contains(secret_title), "leaked title: {err}");

        let mut bad_format = valid_outline();
        bad_format.sections[1].title = secret_title.to_string();
        bad_format.sections[1].format = "table".to_string();
        let err = validate(&bad_format).expect_err("must reject");
        assert!(!err.contains(secret_title), "leaked title: {err}");
        assert!(!err.contains("table"), "leaked format value: {err}");

        let mut echo_title = valid_outline();
        echo_title.sections[1].title = format!("<{secret_title}>");
        let err = validate(&echo_title).expect_err("must reject");
        assert!(!err.contains(secret_title), "leaked title: {err}");
    }
}
