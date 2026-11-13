//! Timestamped transcript evidence for the Topics jobs (specs/0089).
//!
//! The transcript is cut into labelled blocks (`[12:34] Name: text`); the model may cite a
//! time only by one of those labels, and `Evidence::resolve` is the only way a cited time
//! becomes a number. A label that was never shown to the model resolves to nothing, so a
//! hallucinated or reformatted time is dropped rather than stored.

use std::collections::HashMap;

use anyhow::{Context, Result};
use sqlx::SqlitePool;

use super::limits::{BLOCK_SECONDS, EVIDENCE_CHAR_BUDGET};

/// One transcript line: recording-relative start, resolved speaker name, text.
#[derive(Debug, Clone, PartialEq)]
pub struct Segment {
    pub t: Option<f64>,
    pub speaker: Option<String>,
    pub text: String,
}

/// A run of one speaker's consecutive segments, at most `BLOCK_SECONDS` long.
#[derive(Debug, Clone, PartialEq)]
pub struct Block {
    pub t: f64,
    /// `12:34` or `1:02:03`, no brackets.
    pub label: String,
    /// `[12:34] Name: text`.
    pub line: String,
}

/// Transcript text shown to the model plus the labels it may cite.
#[derive(Debug, Clone, Default)]
pub struct Evidence {
    pub text: String,
    labels: HashMap<String, f64>,
}

#[derive(sqlx::FromRow)]
struct SegmentRow {
    audio_start_time: Option<f64>,
    transcript: String,
    speaker: Option<String>,
}

/// The meeting's transcript in recording order, with speaker names where known.
pub async fn load_segments(pool: &SqlitePool, meeting_id: &str) -> Result<Vec<Segment>> {
    let rows = sqlx::query_as::<_, SegmentRow>(
        "SELECT t.audio_start_time, t.transcript,
                NULLIF(TRIM(s.display_name), '') AS speaker
         FROM transcripts t
         LEFT JOIN speakers s ON s.meeting_id = t.meeting_id AND s.speaker_key = t.speaker
         WHERE t.meeting_id = ?
         ORDER BY t.audio_start_time IS NULL, t.audio_start_time, t.timestamp, t.rowid",
    )
    .bind(meeting_id)
    .fetch_all(pool)
    .await
    .context("Couldn't read the transcript")?;
    Ok(rows
        .into_iter()
        .map(|r| Segment {
            t: r.audio_start_time,
            speaker: r.speaker,
            text: r.transcript,
        })
        .collect())
}

fn squash(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

struct Acc {
    t: f64,
    speaker: Option<String>,
    text: String,
}

fn finish(a: Acc) -> Block {
    let label = crate::agent_access::links::clock(a.t);
    let line = match &a.speaker {
        Some(name) => format!("[{label}] {name}: {}", a.text),
        None => format!("[{label}] {}", a.text),
    };
    Block {
        t: a.t,
        label,
        line,
    }
}

/// Merges consecutive same-speaker segments until `BLOCK_SECONDS` have passed since the
/// block started. Untimed segments join the previous block (dropped if there is none).
pub fn blocks(segments: &[Segment]) -> Vec<Block> {
    let mut out = Vec::new();
    let mut cur: Option<Acc> = None;
    for seg in segments {
        let text = squash(&seg.text);
        if text.is_empty() {
            continue;
        }
        let speaker = seg.speaker.as_deref().map(squash).filter(|s| !s.is_empty());
        match seg.t.filter(|t| t.is_finite() && *t >= 0.0) {
            None => {
                if let Some(a) = cur.as_mut() {
                    a.text.push(' ');
                    a.text.push_str(&text);
                }
            }
            Some(t) => {
                let joins = cur
                    .as_ref()
                    .is_some_and(|a| a.speaker == speaker && t - a.t < BLOCK_SECONDS);
                if joins {
                    let a = cur.as_mut().expect("checked");
                    a.text.push(' ');
                    a.text.push_str(&text);
                } else {
                    if let Some(a) = cur.take() {
                        out.push(finish(a));
                    }
                    cur = Some(Acc { t, speaker, text });
                }
            }
        }
    }
    if let Some(a) = cur {
        out.push(finish(a));
    }
    out
}

const STOPWORDS: &[&str] = &[
    "about", "after", "again", "also", "because", "before", "being", "between", "could", "does",
    "doing", "during", "each", "from", "have", "having", "into", "just", "like", "meeting",
    "meetings", "more", "most", "only", "other", "over", "same", "should", "some", "such", "than",
    "that", "their", "them", "then", "there", "these", "they", "this", "those", "through", "topic",
    "under", "very", "what", "when", "where", "which", "while", "will", "with", "would", "your",
];

/// Lowercased full name first, then name/description tokens of 4+ letters minus stopwords.
pub fn topic_terms(name: &str, description: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let full = squash(&name.to_lowercase());
    if !full.is_empty() {
        out.push(full);
    }
    for tok in format!("{name} {description}")
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
    {
        if tok.chars().count() >= 4 && !STOPWORDS.contains(&tok) && !out.iter().any(|t| t == tok) {
            out.push(tok.to_string());
        }
    }
    out
}

impl Evidence {
    /// The time of a cited label (`12:34` or `[12:34]`), only if the model was shown it.
    pub fn resolve(&self, cited: &str) -> Option<f64> {
        let s = cited.trim();
        let s = s.strip_prefix('[').unwrap_or(s);
        let s = s.strip_suffix(']').unwrap_or(s);
        self.labels.get(s.trim()).copied()
    }

    fn render(blocks: &[Block], picked: &[usize]) -> Evidence {
        let mut idx = picked.to_vec();
        idx.sort_unstable();
        idx.dedup();
        let mut text = String::new();
        let mut labels = HashMap::new();
        let mut prev: Option<usize> = None;
        for &i in &idx {
            if prev.is_some_and(|p| i != p + 1) || (prev.is_none() && i != 0) {
                text.push_str("[…]\n");
            }
            text.push_str(&blocks[i].line);
            text.push('\n');
            labels.entry(blocks[i].label.clone()).or_insert(blocks[i].t);
            prev = Some(i);
        }
        if prev.is_some_and(|p| p + 1 < blocks.len()) {
            text.push_str("[…]\n");
        }
        Evidence {
            text: text.trim_end().to_string(),
            labels,
        }
    }
}

/// Cost of one block against the budget, with room for a gap marker beside it.
fn cost(b: &Block) -> usize {
    b.line.len() + 1 + "[…]\n".len()
}

fn hits(b: &Block, terms: &[String]) -> usize {
    let line = b.line.to_lowercase();
    terms.iter().filter(|t| line.contains(t.as_str())).count()
}

/// Everything when it fits; otherwise the first and last 3 blocks, then the best term
/// hits, then an even spread of what is left. Chronological, `[…]` between gaps.
pub fn for_entry(blocks: &[Block], terms: &[String]) -> Evidence {
    let total: usize = blocks.iter().map(|b| b.line.len() + 1).sum();
    if total <= EVIDENCE_CHAR_BUDGET {
        return Evidence::render(blocks, &(0..blocks.len()).collect::<Vec<_>>());
    }
    let n = blocks.len();
    let mut picked: Vec<usize> = Vec::new();
    let mut used = 0usize;
    let mut take = |i: usize, picked: &mut Vec<usize>| -> bool {
        if picked.contains(&i) || used + cost(&blocks[i]) > EVIDENCE_CHAR_BUDGET {
            return false;
        }
        used += cost(&blocks[i]);
        picked.push(i);
        true
    };
    for i in (0..3.min(n)).chain(n.saturating_sub(3)..n) {
        take(i, &mut picked);
    }
    let mut ranked: Vec<(usize, usize)> = (0..n)
        .map(|i| (hits(&blocks[i], terms), i))
        .filter(|(h, _)| *h > 0)
        .collect();
    ranked.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    for (_, i) in ranked {
        take(i, &mut picked);
    }
    // Spread the remainder: repeatedly take the block farthest from any picked one.
    loop {
        let best = (0..n)
            .filter(|i| !picked.contains(i))
            .map(|i| {
                let d = picked.iter().map(|p| p.abs_diff(i)).min().unwrap_or(n);
                (d, i)
            })
            .max_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)));
        match best {
            Some((_, i)) if take(i, &mut picked) => {}
            _ => break,
        }
    }
    Evidence::render(blocks, &picked)
}

/// Blocks with at least one term hit plus a neighbour either side, within budget. Empty
/// text when nothing hits.
pub fn for_mentions(blocks: &[Block], terms: &[String]) -> Evidence {
    let mut ranked: Vec<(usize, usize)> = (0..blocks.len())
        .map(|i| (hits(&blocks[i], terms), i))
        .filter(|(h, _)| *h > 0)
        .collect();
    if ranked.is_empty() {
        return Evidence::default();
    }
    ranked.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    let mut picked: Vec<usize> = Vec::new();
    let mut used = 0usize;
    for (_, i) in ranked {
        let lo = i.saturating_sub(1);
        let hi = (i + 1).min(blocks.len() - 1);
        let want: Vec<usize> = (lo..=hi).filter(|j| !picked.contains(j)).collect();
        let want_cost: usize = want.iter().map(|&j| cost(&blocks[j])).sum();
        if used + want_cost <= EVIDENCE_CHAR_BUDGET {
            used += want_cost;
            picked.extend(want);
        } else if !picked.contains(&i) && used + cost(&blocks[i]) <= EVIDENCE_CHAR_BUDGET {
            used += cost(&blocks[i]);
            picked.push(i);
        }
    }
    Evidence::render(blocks, &picked)
}

#[cfg(test)]
#[path = "evidence_tests.rs"]
mod tests;
