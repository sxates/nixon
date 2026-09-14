//! Per-call-site generation overrides for the built-in (sidecar) provider.
//!
//! Before specs/0053, `llm_client::generate_summary` short-circuited BuiltInAI
//! (`llm_client.rs:234`) *before* reading `max_tokens`/`temperature`/`top_p`, so a
//! call site's sampling intent was silently discarded and every call ran the
//! model's summary profile. Action-item extraction asked for `temperature 0.0`
//! and got `0.5` with `presence_penalty 0.3` — a penalty on exactly the repeated
//! tokens (`{`, `"description"`, `}`) that a JSON array is made of.

use crate::summary::summary_engine::models::SamplingParams;

/// Overrides layered over a model's built-in sampling profile. `Default` changes
/// nothing, so existing callers keep today's behaviour exactly.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct GenerationOptions {
    /// Output cap. `None` keeps `models::DEFAULT_MAX_TOKENS`.
    pub max_tokens: Option<i32>,
    /// Sampling temperature. `None` keeps the model profile's.
    pub temperature: Option<f32>,
    /// GBNF grammar constraining output on the BuiltInAI sidecar. `None` = unconstrained.
    ///
    /// **DO NOT SET THIS TODAY.** `LlamaSampler::grammar` in the pinned `llama-cpp-2
    /// =0.1.146` aborts the *whole process* (`SIGABRT`, exit -6) the moment a
    /// grammar-constrained sampler is used against a real model:
    ///
    /// ```text
    /// llama-grammar.cpp:940: GGML_ASSERT(!stacks.empty()) failed
    /// ```
    ///
    /// This was verified directly against `Qwen3.5-4B-Q4_K_M.gguf` (specs/0053 task 4). It
    /// is not a defect in any particular GBNF: llama.cpp's own bundled reference grammars
    /// (`json.gbnf`, `json_arr.gbnf`, `arithmetic.gbnf`, `chess.gbnf`) and even a trivial
    /// `root ::= "yes"` crash identically through this integration. Since the sidecar is
    /// shared, a crash here takes summarization down with it.
    ///
    /// The field and the sidecar protocol plumbing that reads it are otherwise correct and
    /// tested — this becomes usable the day `llama-cpp-2` is upgraded past the fixed
    /// version. Re-test against a real model (not just unit tests) before wiring any
    /// caller to set this again.
    pub grammar: Option<String>,
    /// JSON-mode intent, independent of `grammar`: drives `response_format` for
    /// OpenAI-compatible providers ([`crate::summary::llm_client::wants_json_mode`]).
    /// Deliberately NOT derived from `grammar.is_some()` — `grammar` cannot be safely set
    /// today (see its doc comment), but JSON mode on hosted providers works fine and must
    /// not be coupled to the broken sidecar path.
    pub json_mode: bool,
    /// Zero the repetition penalties. Required for any format that must repeat
    /// tokens — every JSON array does.
    pub neutral_penalties: bool,
}

impl GenerationOptions {
    /// The profile for structured extraction: greedy, no repetition penalties,
    /// JSON mode. Does NOT set `grammar` — see that field's doc comment for why
    /// (it crashes the sidecar process today).
    pub fn deterministic_json() -> Self {
        Self {
            max_tokens: None,
            temperature: Some(0.0),
            grammar: None,
            json_mode: true,
            neutral_penalties: true,
        }
    }

    /// Layer these overrides onto a model's sampling profile.
    pub fn apply(&self, base: SamplingParams) -> SamplingParams {
        let mut out = base;
        if let Some(t) = self.temperature {
            out.temperature = t;
        }
        if self.neutral_penalties {
            out.presence_penalty = 0.0;
            out.frequency_penalty = 0.0;
            out.repeat_penalty = 1.0;
            out.penalty_last_n = 0;
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn qwen_summary_profile() -> SamplingParams {
        SamplingParams::qwen35_summary(vec!["<|im_end|>".to_string()])
    }

    /// The regression guard: an unset GenerationOptions must not perturb any
    /// existing caller's sampling.
    #[test]
    fn default_options_leave_the_profile_untouched() {
        let base = qwen_summary_profile();
        assert_eq!(GenerationOptions::default().apply(base.clone()), base);
    }

    /// Extraction gets greedy decoding, not the summary profile's 0.5.
    #[test]
    fn deterministic_json_forces_greedy() {
        let out = GenerationOptions::deterministic_json().apply(qwen_summary_profile());
        assert_eq!(out.temperature, 0.0);
    }

    /// The specific bug: presence_penalty 0.3 punishes the repeated keys a JSON
    /// array is made of. All three penalties must be neutral for structured output.
    #[test]
    fn deterministic_json_neutralizes_every_repetition_penalty() {
        let base = qwen_summary_profile();
        assert_eq!(base.presence_penalty, 0.3, "guard: the profile under test");

        let out = GenerationOptions::deterministic_json().apply(base);
        assert_eq!(out.presence_penalty, 0.0);
        assert_eq!(out.frequency_penalty, 0.0);
        assert_eq!(out.repeat_penalty, 1.0);
        assert_eq!(out.penalty_last_n, 0);
    }

    /// Stop tokens are the model's business, not the call site's — overrides
    /// must not clobber them.
    #[test]
    fn overrides_preserve_the_models_stop_tokens() {
        let base = qwen_summary_profile();
        let out = GenerationOptions::deterministic_json().apply(base.clone());
        assert_eq!(out.stop_tokens, base.stop_tokens);
    }

    /// Regression guard (specs/0053 task 4): `deterministic_json` must NEVER set
    /// `grammar` again without a deliberate, re-tested re-enable — `LlamaSampler::grammar`
    /// aborts the sidecar process on the pinned `llama-cpp-2` version (see the field's doc
    /// comment). `json_mode` must still be true so hosted providers keep getting
    /// `response_format`.
    #[test]
    fn deterministic_json_never_sets_grammar() {
        let opts = GenerationOptions::deterministic_json();
        assert!(
            opts.grammar.is_none(),
            "grammar must stay unset: LlamaSampler::grammar crashes the sidecar (llama-grammar.cpp:940)"
        );
        assert!(opts.json_mode, "JSON mode must still be requested");
    }
}
