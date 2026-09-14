//! Transcript sizing → summary depth (specs/0044 WS2) and the script-aware
//! token estimate the chunker/aggregator budget against (spec 0028). Pure,
//! dependency-free; split out of `processor.rs` under the specs/0042 ratchet.

/// [`length_guidance`] band edges (specs/0044 WS2), in [`rough_token_count`]
/// units (which over-estimates): a ~10-minute check-in lands under ~2k tokens;
/// an hour-plus of dense discussion runs past ~12k.
const BRIEF_MEETING_MAX_TOKENS: usize = 2_000;
const DETAILED_MEETING_MIN_TOKENS: usize = 12_000;

/// Depth instruction for the final report, derived from transcript size
/// (specs/0044 WS2). Before this, every meeting got the same template-fill
/// prompt with no length signal at all — so a 10-minute standup and a 90-minute
/// review produced same-sized reports. Pure and unit-tested on the band edges.
pub fn length_guidance(transcript_tokens: usize) -> String {
    let band = if transcript_tokens < BRIEF_MEETING_MAX_TOKENS {
        "This was a SHORT meeting: keep the report brief — a few tight sentences per \
         section, covering only what actually came up. A short check-in deserves a \
         short summary."
    } else if transcript_tokens >= DETAILED_MEETING_MIN_TOKENS {
        "This was a LONG, content-dense meeting: produce a detailed, comprehensive \
         report — preserve every decision, owner, number, and nuance. Here more depth \
         beats brevity."
    } else {
        "This was a medium-length meeting: write a balanced report — cover every \
         substantive topic with enough detail to be useful, without padding."
    };
    let approx_words = transcript_tokens.saturating_mul(3) / 4;
    format!(
        "Scale the report's depth to the meeting's actual content (the transcript is \
         roughly {approx_words} words). {band} Never pad a thin meeting into a long \
         report, and never compress a dense one into a stub."
    )
}

/// Rough token count estimation, script-aware.
///
/// English/Latin text averages ~0.35 tokens/char, but CJK (Chinese/Japanese/Korean)
/// and Thai text is far denser — roughly ~1 token/char for typical subword tokenizers,
/// since each character often maps to its own token or more. Counting those characters
/// at the Latin rate under-estimates by ~3x, which let non-English transcripts overflow
/// a local model's context (the chunker trusts this estimate). We therefore weight
/// dense-script characters at 1 token each and the rest at 0.35, which errs
/// conservatively (slightly over-counts) so chunks stay safely within context.
pub fn rough_token_count(s: &str) -> usize {
    let mut dense = 0usize; // CJK / Thai / other high-density scripts
    let mut other = 0usize;
    for c in s.chars() {
        if is_dense_script_char(c) {
            dense += 1;
        } else {
            other += 1;
        }
    }
    (dense as f64 + other as f64 * 0.35).ceil() as usize
}

/// Whether a character belongs to a script that is roughly one-token-per-character
/// under common LLM tokenizers (CJK ideographs + kana + Hangul + Thai and related
/// dense Southeast/East-Asian blocks). Used only for a conservative token estimate.
fn is_dense_script_char(c: char) -> bool {
    matches!(c as u32,
        0x3040..=0x30FF   // Hiragana + Katakana
        | 0x3400..=0x4DBF // CJK Unified Ideographs Extension A
        | 0x4E00..=0x9FFF // CJK Unified Ideographs
        | 0xF900..=0xFAFF // CJK Compatibility Ideographs
        | 0xAC00..=0xD7AF // Hangul Syllables
        | 0x1100..=0x11FF // Hangul Jamo
        | 0x0E00..=0x0E7F // Thai
        | 0x0E80..=0x0EFF // Lao
        | 0x1000..=0x109F // Myanmar
        | 0x3000..=0x303F // CJK Symbols and Punctuation
        | 0xFF00..=0xFFEF // Halfwidth/Fullwidth Forms
        | 0x20000..=0x2A6DF // CJK Extension B
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn length_guidance_bands_split_at_the_documented_edges() {
        assert!(length_guidance(BRIEF_MEETING_MAX_TOKENS - 1).contains("SHORT meeting"));
        assert!(length_guidance(BRIEF_MEETING_MAX_TOKENS).contains("medium-length meeting"));
        assert!(length_guidance(DETAILED_MEETING_MIN_TOKENS - 1).contains("medium-length meeting"));
        assert!(length_guidance(DETAILED_MEETING_MIN_TOKENS).contains("LONG, content-dense"));
    }

    #[test]
    fn length_guidance_always_carries_the_proportionality_rule() {
        for tokens in [0, 500, 5_000, 50_000] {
            let g = length_guidance(tokens);
            assert!(
                g.contains("Scale the report's depth"),
                "missing rule for {tokens}"
            );
            assert!(
                g.contains("Never pad a thin meeting"),
                "missing rule for {tokens}"
            );
        }
    }

    #[test]
    fn rough_token_count_weights_cjk_higher_than_latin() {
        // 10 Latin chars ~= 4 tokens; 10 CJK chars ~= 10 tokens (denser).
        let latin = rough_token_count("abcdefghij"); // 10 * 0.35 = 3.5 -> 4
        let cjk = rough_token_count("会議録音要約日本語話者一覧"); // 13 CJK chars -> 13
        assert_eq!(latin, 4);
        assert!(cjk >= 13, "CJK undercount: {cjk}");
    }

    #[test]
    fn rough_token_count_matches_legacy_for_pure_latin() {
        // Must be byte-identical to the old `chars * 0.35` for Latin text so English
        // behavior is unchanged.
        let s = "The quick brown fox jumps over the lazy dog.";
        let expected = (s.chars().count() as f64 * 0.35).ceil() as usize;
        assert_eq!(rough_token_count(s), expected);
    }
}
