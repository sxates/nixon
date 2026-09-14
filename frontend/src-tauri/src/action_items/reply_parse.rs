//! Defensive parsing of the extraction reply (specs/0034, extracted from
//! `extractor.rs` by specs/0054 W1).
//!
//! The pipeline is: strip reasoning blocks and a wrapping code fence, prefer an
//! inner fenced block, slice the first `[` … last `]`, then a STRICT `serde_json`
//! parse into [`ActionItemCandidate`]s.
//!
//! Slicing to the array is what lets ONE parser accept both shapes the providers
//! actually produce: the object-wrapped `{"action_items": [...]}` the prompt now
//! asks for (specs/0054 W1 — it is what `response_format: json_object` can
//! actually express), and a bare `[...]`, which hosted providers still return
//! because they treat json mode as a hint. Neither needs a separate code path.
//!
//! Errors never quote model output: it derives from the transcript, and the
//! privacy rule is that content is never logged.

use anyhow::Context;

use crate::action_items::extractor::ActionItemCandidate;
use crate::summary::processor::clean_llm_markdown_output;

/// The body of the first inner fenced block that contains a `[`, if any. Handles the
/// prose-plus-fence pattern (`Here are the items:\n```json\n[…]\n````) where the fence
/// does NOT wrap the whole output (that case is already unwrapped by
/// [`clean_llm_markdown_output`]) — preferring the fenced body keeps a `[` in the
/// surrounding prose from poisoning the bracket slice.
fn inner_fenced_block(text: &str) -> Option<&str> {
    let open = text.find("```")?;
    let body_start = open + text[open..].find('\n')? + 1;
    let body_end = body_start + text[body_start..].find("```")?;
    let body = &text[body_start..body_end];
    body.contains('[').then_some(body)
}

/// Parses one LLM reply into candidates, defensively (specs/0034 risk: small-model JSON
/// discipline): strip reasoning blocks + a wrapping code fence
/// ([`clean_llm_markdown_output`] — `<think>` blocks can themselves contain brackets),
/// prefer an inner fenced block, slice first `[` … last `]`, then STRICT `serde_json`
/// into [`ActionItemCandidate`]s. Never quotes model output in errors (it derives from
/// the transcript; privacy rule: never log content).
pub(crate) fn parse_candidates(raw: &str) -> anyhow::Result<Vec<ActionItemCandidate>> {
    let cleaned = clean_llm_markdown_output(raw);
    let body = inner_fenced_block(&cleaned).unwrap_or(&cleaned);
    let start = body
        .find('[')
        .context("model reply contains no JSON array opening bracket")?;
    let end = body
        .rfind(']')
        .context("model reply contains no JSON array closing bracket")?;
    anyhow::ensure!(
        start < end,
        "model reply's brackets do not enclose a JSON array"
    );
    serde_json::from_str::<Vec<ActionItemCandidate>>(&body[start..=end])
        .context("model reply is not a JSON array of {description, assignee, due} objects")
}

/// A content-free description of an unparseable reply, for the failure log.
///
/// Reports only the length and the first non-whitespace character — never any of
/// the reply itself, which derives from the meeting transcript. Those two facts
/// are enough to tell the failure modes apart in a user's log: a `{` start means
/// the model produced an object where an array was wanted (specs/0054 W1), a
/// length at the provider's token cap means truncation instead, and anything else
/// means the model ignored the output contract.
pub(crate) fn describe_shape(reply: &str) -> String {
    let first = reply
        .chars()
        .find(|c| !c.is_whitespace())
        .map(|c| {
            if c.is_alphanumeric() {
                "alphanumeric (prose?)".to_string()
            } else {
                format!("{c:?}")
            }
        })
        .unwrap_or_else(|| "empty".to_string());
    format!("{} chars, starts with {}", reply.len(), first)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The failure log must never leak reply content — it derives from the
    /// transcript, and this string goes to `~/Library/Logs/`.
    #[test]
    fn describe_shape_never_echoes_reply_content() {
        let secret = "Acquisition of Northwind closes Tuesday";
        let described = describe_shape(&format!("  {{\"x\": \"{secret}\"}}"));
        for word in secret.split_whitespace() {
            assert!(
                !described.contains(word),
                "leaked {word:?} in {described:?}"
            );
        }
        assert!(described.contains("starts with"));
    }

    /// The three failure modes must be distinguishable from the log line alone.
    #[test]
    fn describe_shape_separates_the_failure_modes() {
        assert!(describe_shape("{\"description\": \"x\"}").contains("\'{\'"));
        assert!(describe_shape("[{}]").contains("\'[\'"));
        assert!(describe_shape("Here are the items:").contains("prose"));
        assert!(describe_shape("   ").contains("empty"));
        assert!(describe_shape("abc").contains("3 chars"));
    }
}
