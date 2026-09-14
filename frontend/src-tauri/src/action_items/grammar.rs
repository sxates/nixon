//! GBNF grammar for action-item extraction output (specs/0053 W1) — **NOT WIRED**.
//!
//! `parse_candidates` (below) is the live, primary defence for extraction output. The
//! [`ACTION_ITEMS_GBNF`] grammar is kept as source but is **not passed to any generation
//! call** — `extractor.rs` builds `GenerationOptions::deterministic_json()`, which never
//! sets `grammar`.
//!
//! ## Why: it crashes the sidecar process
//!
//! specs/0053 task 4 drove the `llama-helper` sidecar directly against the real
//! `Qwen3.5-4B-Q4_K_M.gguf` model with a grammar-constrained sampler. `LlamaSampler::grammar`
//! in the pinned `llama-cpp-2 =0.1.146` aborts the whole process:
//!
//! ```text
//! llama-grammar.cpp:940: GGML_ASSERT(!stacks.empty()) failed   → SIGABRT (exit -6)
//! ```
//!
//! This is not a defect in this GBNF specifically: llama.cpp's own bundled reference
//! grammars (`json.gbnf`, `json_arr.gbnf`, `arithmetic.gbnf`, `chess.gbnf`) and even a
//! trivial `root ::= "yes"` crash identically through this integration. Since the sidecar
//! is shared, shipping this would take summarization down with it, not just extraction.
//!
//! Separately, this grammar as originally written never even parsed: llama.cpp's GBNF
//! parser ends a rule at the newline, so the original multi-line `item` rule was a syntax
//! error (`parse: error parsing grammar: expecting name`). A failed parse does not surface
//! as an error at the call site — it silently produces empty/unconstrained output. The
//! `item` rule below has been fixed to a single line so the constant is at least correct
//! if revived, but it is still unused.
//!
//! ## Rules for anyone re-enabling this
//!
//! 1. Every GBNF rule must be on ONE line — llama.cpp's parser ends a rule at the newline.
//! 2. Re-test against a REAL model (not just unit tests) before wiring a caller to set
//!    `GenerationOptions.grammar` again. Unit tests here only check the grammar text and
//!    `parse_candidates`; they cannot catch a sidecar-process abort.
//!
//! Before this grammar (wired or not), nothing constrained the model's output:
//! `parse_candidates` is defensive but purely post-hoc, and can only recover structure that
//! is present. On the built-in 4B model, extraction failed with "model reply contains no
//! JSON array opening bracket" — the model emitted prose. `parse_candidates` remains the
//! actual fix for that; this grammar was meant to be an additional, machine-enforced layer
//! on top of it but cannot be used until `llama-cpp-2` is upgraded past the crash above.
//!
//! This grammar is the machine-enforced form of the `ActionItemCandidate` serde
//! contract: an array (possibly empty) of objects with exactly `description`
//! (string), `assignee` (string or null) and `due` (string or null).

/// **NOT WIRED — see the module doc comment above before using this.** Root rule name is
/// `root`, as `LlamaSampler::grammar` expects, but nothing passes this to a generation call
/// today: `LlamaSampler::grammar` aborts the sidecar process on the pinned `llama-cpp-2`
/// version (`llama-grammar.cpp:940: GGML_ASSERT(!stacks.empty()) failed`, verified against
/// the real model). Kept `pub` (not `#[allow(dead_code)]`) so it stays compiled and
/// available the day it's safe to re-enable.
///
/// `char` excludes raw control bytes (`\x00`-`\x1F`), not just `"` and `\` —
/// `serde_json` (which `parse_candidates` uses) rejects a literal control
/// character inside a string with `ControlCharacterWhileParsingString`, so a
/// model emitting e.g. a literal newline in a description would otherwise
/// produce grammar-valid output that still fails to parse: the exact
/// grammar/parser drift this task exists to eliminate, just in the permissive
/// direction. (The reference `json.gbnf` shipped in `llama-cpp-2` has this
/// same gap — this isn't an obvious mistake to make.)
///
/// Every rule is on a single line — llama.cpp's GBNF parser ends a rule at the newline, so
/// the original multi-line `item` rule below was a silent syntax error.
pub const ACTION_ITEMS_GBNF: &str = r#"
root        ::= ws "[" ws (item (ws "," ws item)*)? ws "]" ws
item        ::= "{" ws "\"description\"" ws ":" ws string ws "," ws "\"assignee\"" ws ":" ws strornull ws "," ws "\"due\"" ws ":" ws strornull ws "}"
strornull   ::= string | "null"
string      ::= "\"" char* "\""
char        ::= [^"\\\x00-\x1F] | "\\" ["\\/bfnrt] | "\\u" hex hex hex hex
hex         ::= [0-9a-fA-F]
ws          ::= [ \t\n\r]*
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action_items::extractor::ActionItemCandidate;
    use crate::action_items::reply_parse::parse_candidates;

    /// The canonical shape, moved here from extractor.rs (specs/0053 — that file
    /// is ratchet-allowlisted and may only shrink).
    const CLEAN_ARRAY: &str = r#"[{"description": "Send the deck to Alice", "assignee": "Alice", "due": "Friday"}, {"description": "Book the room", "assignee": "me", "due": null}]"#;

    /// The grammar must describe exactly what the parser accepts. If these two
    /// drift apart, grammar-constrained output would still fail to parse.
    #[test]
    fn the_canonical_array_parses() {
        let parsed = parse_candidates(CLEAN_ARRAY).expect("canonical fixture must parse");
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].description, "Send the deck to Alice");
        assert_eq!(parsed[0].assignee.as_deref(), Some("Alice"));
        assert_eq!(parsed[1].due, None);
    }

    /// A model that wraps the array in a fenced code block — tagged or untagged —
    /// must still parse; `clean_llm_markdown_output` + the fenced-block preference
    /// in `parse_candidates` handle this before the bracket slice ever runs.
    #[test]
    fn parse_code_fenced_json() {
        let fenced = format!("```json\n{CLEAN_ARRAY}\n```");
        assert_eq!(parse_candidates(&fenced).unwrap().len(), 2);
        // Untagged fence too.
        let untagged = format!("```\n{CLEAN_ARRAY}\n```\n");
        assert_eq!(parse_candidates(&untagged).unwrap().len(), 2);
    }

    /// The prose contains a bracket ("[2 total]") that would poison a naive
    /// first-`[` slice; the inner fenced block must win. This is the fallback path
    /// for providers that honour neither `grammar` nor `response_format` (Ollama,
    /// custom OpenAI-compatible endpoints) and still wrap the array in prose.
    #[test]
    fn parse_prose_with_inner_fence_prefers_fenced_body() {
        let reply = format!(
            "I identified the following tasks [2 total]:\n\n```json\n{CLEAN_ARRAY}\n```\n\nBoth came from the Action Items section."
        );
        assert_eq!(parse_candidates(&reply).unwrap().len(), 2);
    }

    /// Ollama-typical: a `<think>` block that itself contains brackets, then the
    /// array. `clean_llm_markdown_output` must strip the reasoning block before the
    /// bracket slice runs, or the think-block's brackets would poison it.
    #[test]
    fn parse_thinking_block_with_brackets_is_stripped() {
        let reply = format!(
            "<think>The summary lists tasks[3]... wait, only [2] are commitments.</think>\n{CLEAN_ARRAY}"
        );
        assert_eq!(parse_candidates(&reply).unwrap().len(), 2);
    }

    /// A model that wraps the array in an object still yields the array via the
    /// first-`[`…last-`]` slice — a shape the grammar itself can't produce (`root`
    /// always starts at `[`), so this exercises the parser's own backstop for
    /// providers that ignore the grammar/`response_format` hint entirely.
    #[test]
    fn parse_object_wrapped_array_recovers() {
        let wrapped = format!(r#"{{"action_items": {CLEAN_ARRAY}}}"#);
        assert_eq!(parse_candidates(&wrapped).unwrap().len(), 2);
    }

    /// specs/0054 W1 regression. Reproduced against `gemma4:26b` via Ollama: with
    /// `response_format: {"type":"json_object"}` (which `deterministic_json` sets for
    /// every OpenAI-compatible provider) and the OLD bare-array prompt, the model
    /// answered with a single bare object — no array anywhere. It parses as neither
    /// shape, and because extraction runs at `temperature: 0.0` the retry returned the
    /// identical bytes, so both attempts failed and the user saw "extraction failed".
    ///
    /// The fix is the prompt (now object-wrapped, so the model's only legal answer is
    /// also the one we want). This test pins the failure shape so that if the contract
    /// ever regresses to a bare array, it fails here rather than in the user's meeting.
    #[test]
    fn a_single_bare_object_is_rejected_not_silently_accepted() {
        let collapsed = r#"{"description": "Send the deck", "assignee": "me", "due": "Friday"}"#;
        let err = parse_candidates(collapsed)
            .expect_err("a single object is not a candidate list — it silently loses items");
        assert!(
            format!("{err:#}").contains("opening bracket"),
            "expected the no-array diagnostic, got: {err:#}"
        );
    }

    /// The other half of the same reproduction: a bare array must KEEP parsing, because
    /// hosted providers treat json mode as a hint and still return one.
    #[test]
    fn a_bare_array_still_parses_after_the_object_contract_change() {
        assert_eq!(parse_candidates(CLEAN_ARRAY).unwrap().len(), 2);
    }

    /// Malformed or wrong-shape JSON must still be rejected by the strict-serde
    /// backstop, independent of the grammar: a truncated array (output-cap
    /// cutoff), and syntactically valid JSON in the wrong shape.
    #[test]
    fn parse_rejects_malformed_and_wrong_shape_json() {
        // Truncated array (e.g. output-cap cutoff).
        assert!(parse_candidates(r#"[{"description": "Send the deck", "assignee""#).is_err());
        // Right brackets, wrong element shape → strict serde failure.
        assert!(parse_candidates("[1, 2, 3]").is_err());
        assert!(parse_candidates(r#"[{"task": "missing description key"}]"#).is_err());
    }

    /// specs/0053 grammar/parser drift check: a literal control character (here, a
    /// raw newline) inside a description is exactly what `char` in the grammar must
    /// now exclude — `serde_json` rejects it with
    /// `ControlCharacterWhileParsingString`, so grammar-valid output containing one
    /// would still fail to parse. This pins the parser side of that guarantee; the
    /// grammar's `[^"\\\x00-\x1F]` exclusion is the other half.
    #[test]
    fn raw_control_character_in_a_string_is_rejected() {
        let with_raw_newline =
            "[{\"description\": \"Send the\ndeck\", \"assignee\": null, \"due\": null}]";
        let err = parse_candidates(with_raw_newline).expect_err("raw control char must not parse");
        assert!(
            format!("{err:#}").contains("control character"),
            "unexpected error: {err:#}"
        );
    }

    /// An empty array is a legitimate outcome (no action items in this meeting)
    /// and the grammar permits it — critical, because forcing a non-empty array
    /// would make the model invent commitments.
    #[test]
    fn an_empty_array_is_valid_output() {
        assert!(parse_candidates("[]").expect("[] must parse").is_empty());
    }

    /// The exact reported production failure: prose with no bracket at all.
    /// Still an error — the grammar prevents it upstream, the parser remains
    /// the backstop for providers that honour neither grammar nor JSON mode.
    #[test]
    fn prose_without_a_bracket_is_still_rejected() {
        let err = parse_candidates("There are no action items in this meeting.")
            .expect_err("prose must not parse");
        assert!(
            format!("{err:#}").contains("no JSON array opening bracket"),
            "unexpected error: {err:#}"
        );
    }

    /// Round-trip: the serde contract the grammar mirrors.
    #[test]
    fn grammar_shape_matches_the_candidate_struct() {
        let candidate = ActionItemCandidate {
            description: "Book the room".to_string(),
            assignee: None,
            due: None,
        };
        let json = serde_json::to_string(&std::slice::from_ref(&candidate)).unwrap();
        assert_eq!(parse_candidates(&json).unwrap(), vec![candidate]);
    }

    /// Guards against an accidental edit that drops the root rule or a key.
    #[test]
    fn grammar_declares_a_root_and_all_three_keys() {
        assert!(ACTION_ITEMS_GBNF.contains("root        ::="));
        for key in ["description", "assignee", "due"] {
            assert!(
                ACTION_ITEMS_GBNF.contains(&format!("\\\"{key}\\\"")),
                "grammar is missing the {key} key"
            );
        }
    }
}
