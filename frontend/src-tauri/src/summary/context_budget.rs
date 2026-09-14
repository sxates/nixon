//! Context-window budgeting for local (Ollama) models (specs/0052).
//!
//! The budget decides single-pass vs. map-reduce chunking — it does not allocate anything.
//! Getting it wrong is silent: too large and the chunker never engages, so a long meeting is
//! context-shifted away without a warning.
//!
//! Nixon used to size it from `/api/show`'s `<family>.context_length`, which is the model's
//! ARCHITECTURAL maximum — 262,144 for gemma4. What the server actually allocates is
//! `OLLAMA_CONTEXT_LENGTH` (or Ollama's VRAM-based default), commonly 64k. Budgeting from the
//! architectural max meant believing there was 4x the available room.

/// Budget floor when the served context is unknown (model idle and nothing cached). Small
/// enough to be safe on any endpoint; the chunking path handles the rest.
pub const CONSERVATIVE_CONTEXT_FLOOR: usize = 8_192;

/// Tokens reserved for prompt scaffolding — unchanged from the original implementation.
const CONTEXT_RESERVE: usize = 300;

/// The usable budget: never more than the server actually serves.
///
/// `served` comes from [`crate::ollama::served_context`]. `None` means we could not learn it
/// (model idle, nothing cached, or an Ollama build that omits the field), in which case we
/// take the conservative floor rather than guessing high — an over-large budget silently
/// truncates, an under-large one merely chunks more than necessary.
pub fn clamp_context_budget(arch_max: usize, served: Option<usize>) -> usize {
    match served {
        Some(served) => arch_max.min(served).saturating_sub(CONTEXT_RESERVE).max(1),
        None => CONSERVATIVE_CONTEXT_FLOOR,
    }
}

/// Resolve the Ollama budget end-to-end: read what the server serves, clamp the model's
/// architectural max to it, and log both so a wrong reading is diagnosable from the log.
pub async fn ollama_context_budget(
    arch_max: usize,
    model_name: &str,
    endpoint: Option<&str>,
) -> usize {
    let served = crate::ollama::served_context::served_context(model_name, endpoint).await;
    let budget = clamp_context_budget(arch_max, served);
    tracing::info!(
        "✓ Context budget for {}: {} tokens (architectural {}, served {:?})",
        model_name,
        budget,
        arch_max,
        served
    );
    budget
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamps_to_the_served_context_when_smaller() {
        // The real bug: gemma4 reports a 262,144 architectural max while the server
        // serves 65,536. Budgeting from the arch max disabled chunking entirely.
        assert_eq!(clamp_context_budget(262_144, Some(65_536)), 65_236);
    }

    #[test]
    fn uses_the_architectural_max_when_it_is_smaller() {
        assert_eq!(clamp_context_budget(8_192, Some(65_536)), 7_892);
    }

    #[test]
    fn falls_back_to_the_conservative_floor_when_served_is_unknown() {
        assert_eq!(
            clamp_context_budget(262_144, None),
            CONSERVATIVE_CONTEXT_FLOOR
        );
    }

    #[test]
    fn never_underflows_on_a_tiny_architectural_max() {
        assert_eq!(clamp_context_budget(100, Some(100)), 1);
    }
}
