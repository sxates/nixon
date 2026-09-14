//! Sampler-chain construction for the generation loop, including optional
//! GBNF grammar-constrained decoding (specs/0053 W1).
//!
//! Extracted from `main.rs` so grammar support could land without pushing that
//! file past the 800-line limit (`scripts/check-file-size.sh`).

use anyhow::{Context, Result};
use llama_cpp_2::model::LlamaModel;
use llama_cpp_2::sampling::LlamaSampler;

use crate::SamplingConfig;

/// The samplers a chain is built from, in application order. Returned separately
/// from the built chain so the ordering contract is unit-testable without a
/// loaded model (`LlamaSampler` is opaque and not introspectable).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SamplerStage {
    Grammar,
    Penalties,
    TopK,
    TopP,
    Temp,
    Dist,
    Greedy,
}

/// The stage order for a given configuration. Pure, so the ordering invariant is
/// testable; `build_sampler` walks this list to construct the real chain.
pub fn sampler_stages(sampling: SamplingConfig, has_grammar: bool) -> Vec<SamplerStage> {
    let mut stages = Vec::new();
    if has_grammar {
        stages.push(SamplerStage::Grammar);
    }
    if sampling.uses_penalties() {
        stages.push(SamplerStage::Penalties);
    }
    if sampling.temperature <= 0.0 {
        stages.push(SamplerStage::Greedy);
    } else {
        stages.push(SamplerStage::TopK);
        stages.push(SamplerStage::TopP);
        stages.push(SamplerStage::Temp);
        stages.push(SamplerStage::Dist);
    }
    stages
}

/// Build the sampler chain for one generation.
///
/// When `grammar` is `Some`, a GBNF grammar sampler leads the chain, making
/// output that does not parse under the grammar unrepresentable rather than
/// merely discouraged. When `None`, the chain is byte-for-byte what `main.rs`
/// built before specs/0053.
pub fn build_sampler(
    model: &LlamaModel,
    sampling: SamplingConfig,
    seed: u32,
    grammar: Option<&str>,
) -> Result<LlamaSampler> {
    let mut chain: Vec<LlamaSampler> = Vec::new();

    for stage in sampler_stages(sampling, grammar.is_some()) {
        let sampler = match stage {
            SamplerStage::Grammar => {
                let gbnf = grammar.expect("stage list only yields Grammar when Some");
                LlamaSampler::grammar(model, gbnf, "root")
                    // GrammarError is not std::error::Error in this version;
                    // render it so anyhow can carry it.
                    .map_err(|e| anyhow::anyhow!("invalid GBNF grammar: {e:?}"))
                    .context("failed to build the grammar sampler")?
            }
            SamplerStage::Penalties => LlamaSampler::penalties(
                sampling.penalty_last_n,
                sampling.repeat_penalty,
                sampling.frequency_penalty,
                sampling.presence_penalty,
            ),
            SamplerStage::TopK => LlamaSampler::top_k(sampling.top_k),
            SamplerStage::TopP => LlamaSampler::top_p(sampling.top_p, 1),
            SamplerStage::Temp => LlamaSampler::temp(sampling.temperature),
            SamplerStage::Dist => LlamaSampler::dist(seed),
            SamplerStage::Greedy => LlamaSampler::greedy(),
        };
        chain.push(sampler);
    }

    Ok(LlamaSampler::chain_simple(chain))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn greedy_no_penalties() -> SamplingConfig {
        SamplingConfig::from_request(Some(0.0), None, None, None, None, None, None)
    }

    fn sampled_with_penalties() -> SamplingConfig {
        SamplingConfig::from_request(
            Some(0.5),
            Some(20),
            Some(0.8),
            Some(0.3),
            Some(0.0),
            Some(1.05),
            Some(256),
        )
    }

    /// The load-bearing invariant: the grammar masks logits BEFORE anything else
    /// reshapes them.
    #[test]
    fn grammar_is_always_the_first_stage() {
        for sampling in [greedy_no_penalties(), sampled_with_penalties()] {
            let stages = sampler_stages(sampling, true);
            assert_eq!(
                stages.first(),
                Some(&SamplerStage::Grammar),
                "grammar must lead the chain for {sampling:?}"
            );
        }
    }

    /// No grammar requested => the chain is exactly what main.rs built before
    /// specs/0053. This is the regression guard for every existing caller.
    #[test]
    fn without_grammar_the_chain_is_unchanged() {
        assert_eq!(
            sampler_stages(greedy_no_penalties(), false),
            vec![SamplerStage::Greedy]
        );
        assert_eq!(
            sampler_stages(sampled_with_penalties(), false),
            vec![
                SamplerStage::Penalties,
                SamplerStage::TopK,
                SamplerStage::TopP,
                SamplerStage::Temp,
                SamplerStage::Dist,
            ]
        );
    }

    /// Extraction's profile: temperature 0 and penalties neutralized, so the
    /// chain is grammar + greedy and nothing else can perturb the JSON.
    #[test]
    fn extraction_profile_is_grammar_then_greedy() {
        let extraction = SamplingConfig::from_request(
            Some(0.0),
            None,
            None,
            Some(0.0),
            Some(0.0),
            Some(1.0),
            Some(0),
        );
        assert_eq!(
            sampler_stages(extraction, true),
            vec![SamplerStage::Grammar, SamplerStage::Greedy]
        );
    }
}
