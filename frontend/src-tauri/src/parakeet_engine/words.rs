//! Token → word grouping for Parakeet's per-token timestamps.
//!
//! `ParakeetModel::decode_tokens` (see `model.rs`) already replaces the SentencePiece
//! word-start marker `\u{2581}` ("▁") with a literal leading space `' '` when it builds
//! the vocabulary (`load_vocab`, `model.rs:171`). So by the time a token reaches
//! `TimestampedResult::tokens`, a token that *starts a new word* begins with a literal
//! `' '` character (e.g. `" hello"`), while a token that *continues* the previous word
//! has no leading space (e.g. `"llo"` continuing `" he"` -> `"hello"`). This mirrors
//! exactly what `DECODE_SPACE_RE` (`model.rs:20-21`) does when it reconstructs the full
//! decoded `text` string from the joined tokens — we're just doing it at token
//! granularity instead of on the joined string, so we can keep per-word timestamps.
//!
//! Word `start` = the timestamp of the word's first token. Word `end` = the timestamp
//! of the word's *last* token (Parakeet's timestamps mark token emission time, not a
//! token duration, so this is the latest known time at which the word was still being
//! decoded — there's no explicit "word end" signal available from the model).
//!
//! A token that is *only* a space marker (i.e. empty after stripping the leading
//! space — this happens for e.g. standalone punctuation-adjacent separators) closes
//! out the current word but does not itself start a new one; the next non-empty token
//! starts the next word.

use crate::parakeet_engine::parakeet_engine::ParakeetEngine;
use anyhow::{anyhow, Result};

/// A single word with its start/end time (seconds) in the source audio.
#[derive(Debug, Clone, PartialEq)]
pub struct WordStamp {
    pub text: String,
    pub start: f32,
    pub end: f32,
}

/// A transcription result carrying both the full text and per-word timestamps.
#[derive(Debug, Clone, PartialEq)]
pub struct TranscribedWords {
    pub text: String,
    pub words: Vec<WordStamp>,
}

/// Group Parakeet's per-token strings + per-token timestamps into per-word
/// `WordStamp`s. Pure function — see module docs for the grouping rule.
pub fn group_tokens_to_words(tokens: &[String], timestamps: &[f32]) -> Vec<WordStamp> {
    let mut words: Vec<WordStamp> = Vec::new();
    let mut current: Option<WordStamp> = None;

    for (token, &ts) in tokens.iter().zip(timestamps.iter()) {
        let starts_new_word = token.starts_with(' ');
        let piece = token.trim_start_matches(' ');

        if piece.is_empty() {
            // Pure separator token (e.g. a lone marker): close the current word,
            // but don't start a new one — the next non-empty token does that.
            if let Some(word) = current.take() {
                words.push(word);
            }
            continue;
        }

        if starts_new_word || current.is_none() {
            if let Some(word) = current.take() {
                words.push(word);
            }
            current = Some(WordStamp {
                text: piece.to_string(),
                start: ts,
                end: ts,
            });
        } else if let Some(word) = current.as_mut() {
            word.text.push_str(piece);
            word.end = ts;
        }
    }

    if let Some(word) = current.take() {
        words.push(word);
    }

    words
}

impl ParakeetEngine {
    /// Transcribe audio and return per-word timestamps alongside the text.
    ///
    /// Kept separate from `transcribe_audio` (which stays `Result<String>`) so the
    /// existing live/STT-lock call sites are untouched — this is a new, additive
    /// entry point for consumers that need word-level timing (e.g. future
    /// word-exact splits).
    pub async fn transcribe_audio_timestamped(
        &self,
        audio_data: Vec<f32>,
    ) -> Result<TranscribedWords> {
        let _inference = crate::audio::stt_lock::acquire_inference_lock().await;
        let mut model_guard = self.current_model.write().await;
        let model = model_guard
            .as_mut()
            .ok_or_else(|| anyhow!("No Parakeet model loaded. Please load a model first."))?;

        let duration_seconds = audio_data.len() as f64 / 16000.0; // Assuming 16kHz
        log::debug!(
            "Parakeet transcribing {} samples ({:.1}s duration) with word timestamps",
            audio_data.len(),
            duration_seconds
        );

        let result = model
            .transcribe_samples(audio_data)
            .map_err(|e| anyhow!("Parakeet transcription failed: {}", e))?;

        let words = group_tokens_to_words(&result.tokens, &result.timestamps);

        log::debug!(
            "Parakeet transcription result: '{}' ({} words)",
            result.text,
            words.len()
        );

        Ok(TranscribedWords {
            text: result.text,
            words,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{group_tokens_to_words, WordStamp};

    #[test]
    fn groups_subword_tokens_into_words_on_space_prefix() {
        // Parakeet's vocab loader replaces the SentencePiece word-start marker
        // ('\u{2581}' / "▁") with a literal leading space before tokens ever reach
        // TimestampedResult::tokens (model.rs load_vocab). So the real per-token
        // marker is a leading ' ', not "▁".
        let tokens = vec![" he".to_string(), "llo".to_string(), " world".to_string()];
        let ts = vec![0.0f32, 0.1, 0.5];
        let words = group_tokens_to_words(&tokens, &ts);
        assert_eq!(
            words.iter().map(|w| w.text.as_str()).collect::<Vec<_>>(),
            vec!["hello", "world"]
        );
        assert!((words[0].start - 0.0).abs() < 1e-6 && (words[0].end - 0.1).abs() < 1e-6);
        assert!((words[1].start - 0.5).abs() < 1e-6 && (words[1].end - 0.5).abs() < 1e-6);
    }

    #[test]
    fn empty_tokens_yield_no_words() {
        assert!(group_tokens_to_words(&[], &[]).is_empty());
    }

    #[test]
    fn single_token_word_with_no_leading_space_still_forms_a_word() {
        // First token in a sequence may have no leading space (e.g. mid-utterance
        // continuation from a prior chunk boundary); it should still start a word.
        let tokens = vec!["hi".to_string()];
        let ts = vec![1.25f32];
        let words = group_tokens_to_words(&tokens, &ts);
        assert_eq!(
            words,
            vec![WordStamp {
                text: "hi".to_string(),
                start: 1.25,
                end: 1.25
            }]
        );
    }

    #[test]
    fn multi_token_word_uses_first_token_start_and_last_token_end() {
        let tokens = vec![
            " un".to_string(),
            "be".to_string(),
            "liev".to_string(),
            "able".to_string(),
        ];
        let ts = vec![2.0f32, 2.08, 2.16, 2.24];
        let words = group_tokens_to_words(&tokens, &ts);
        assert_eq!(words.len(), 1);
        assert_eq!(words[0].text, "unbelievable");
        assert!((words[0].start - 2.0).abs() < 1e-6);
        assert!((words[0].end - 2.24).abs() < 1e-6);
    }

    #[test]
    fn pure_separator_token_closes_current_word_without_starting_a_new_one() {
        // A token that is only a space marker (empty after stripping the leading
        // space) should not itself become a zero-length "word".
        let tokens = vec![" hi".to_string(), " ".to_string(), " there".to_string()];
        let ts = vec![0.0f32, 0.2, 0.3];
        let words = group_tokens_to_words(&tokens, &ts);
        assert_eq!(
            words.iter().map(|w| w.text.as_str()).collect::<Vec<_>>(),
            vec!["hi", "there"]
        );
    }
}
