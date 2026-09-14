//! Ask-AI prompt pair + citation post-processing (specs/0035 task 4).
//!
//! The Ask-AI consumer of the aggregation engine is exactly this: an
//! [`AggregationPrompt`] built by [`ask_ai_prompt`] plus the citation
//! post-processing in [`postprocess_citations`]. The engine stays
//! consumer-agnostic — it returns the reduce output raw with a simple
//! `[M#]`-presence `cited` check, and the Ask-AI IPC path (task 5) refines the
//! answer through `postprocess_citations` before emitting it to the frontend.
//!
//! Prompt design intent (documented per the treat-prompts-as-code rule):
//!
//! - **Map = extraction, the easy task.** These prompts must run on everything
//!   from Claude to a 3B Ollama model. Multi-document synthesis with citations
//!   is near the ceiling for small local models, so the per-meeting map stage
//!   is deliberately the load-bearing step: extract verbatim, don't answer,
//!   don't synthesize. A weak model that merely quotes relevant lines still
//!   feeds the reduce stage well.
//! - **`NOTHING RELEVANT` is a machine-readable sentinel**, and we filter it
//!   *before* the reduce call (in the `reduce_user` closure) rather than only
//!   relying on the model to ignore it — cheaper in tokens and deterministic.
//!   The reduce system prompt still instructs the model to ignore any
//!   near-miss variants that survive the strict filter (belt and braces).
//! - **The citation contract is enforced twice**: stated with one explicit
//!   example in the reduce system prompt (small models follow examples far
//!   better than abstract rules), then validated/normalized mechanically by
//!   `postprocess_citations` so the UI never renders an out-of-range chip.

use crate::aggregation::engine::{AggregationPrompt, SourceMeeting};
use crate::aggregation::gather::{DocSource, MeetingDoc};

// ---------------------------------------------------------------------------
// Prompts
// ---------------------------------------------------------------------------

/// Map stage: per-meeting extraction. No answering, no synthesis — the model
/// sees ONE meeting and pulls out everything relevant to the question,
/// verbatim quotes preferred, or emits exactly `NOTHING RELEVANT`.
const MAP_SYSTEM: &str = "\
You extract relevant material from the record of ONE meeting. You will be given \
the meeting's title, date, and content, plus a question.

Your only job is extraction. Do NOT answer the question, do NOT summarize the \
whole meeting, do NOT add opinions or conclusions.

Rules:
- Copy out every part of the meeting content that could help answer the question.
- Prefer verbatim quotes. Keep speaker names, dates, and numbers exactly as written.
- Include concrete specifics: decisions made, action items, owners, deadlines, figures.
- Output a plain list of extracts, one per line. Nothing else.
- If nothing in this meeting relates to the question, output exactly this single \
line and nothing more:
NOTHING RELEVANT";

/// Reduce stage: QA synthesis over the numbered sources, with the `[M#]`
/// citation contract and an explicit not-found behavior.
const REDUCE_SYSTEM: &str = "\
You answer a question using excerpts from several meetings. Each meeting source \
is numbered with a marker like [M1], [M2], [M3] at the start of its section.

Rules:
- Answer the question directly in markdown. Be concise: short paragraphs or \
bullet points, whichever fits the answer.
- EVERY factual claim must end with the marker(s) of the meeting(s) it came \
from. Use one marker per source, written back to back for multiple sources.
  Example: The team decided to launch in March [M2]. Pricing came up in two \
meetings and was raised both times [M1][M3].
- Only use markers that appear in the sources below. Never invent a marker.
- Ignore any source section that says NOTHING RELEVANT.
- If the sources do not contain the answer, reply with a single short sentence \
saying the answer was not found in the selected meetings. Do not guess and do \
not use outside knowledge.";

/// Shown to the reduce model when every map output was `NOTHING RELEVANT` (the
/// filter removed all sections), so the prompt stays well-formed and the model
/// takes the not-found path.
const EMPTY_SOURCES_NOTICE: &str =
    "(None of the selected meetings contained anything relevant to the question.)";

/// Builds the Ask-AI prompt pair for one question.
///
/// Provider-agnostic: no model-specific formatting, no assumptions beyond
/// "instruction-following chat model with a system prompt". The engine numbers
/// docs/map-outputs itself; these closures never emit `[M#]` markers of their
/// own at map stage.
pub fn ask_ai_prompt(question: &str) -> AggregationPrompt {
    let map_question = question.trim().to_string();
    let reduce_question = map_question.clone();

    AggregationPrompt {
        map_system: MAP_SYSTEM.to_string(),
        map_user: Box::new(move |doc: &MeetingDoc| {
            format!(
                "Question: {question}\n\n\
                 Meeting: \"{title}\"\n\
                 Date: {date}\n\
                 Content source: {source}\n\n\
                 Meeting content:\n{content}\n\n\
                 Extract everything relevant to the question above, verbatim where \
                 possible. If nothing is relevant, output exactly: NOTHING RELEVANT",
                question = map_question,
                title = doc.title,
                date = doc.created_at,
                source = source_label(doc.source),
                content = doc.full_text(),
            )
        }),
        reduce_system: REDUCE_SYSTEM.to_string(),
        reduce_user: Box::new(move |block: &str| {
            let filtered = filter_nothing_relevant(block);
            let sources = if filtered.is_empty() {
                EMPTY_SOURCES_NOTICE
            } else {
                filtered.as_str()
            };
            format!(
                "Question: {question}\n\n\
                 Numbered meeting sources:\n\n{sources}\n\n\
                 Answer the question using only these sources, citing every claim \
                 with its [M#] marker. If they do not answer it, say the answer was \
                 not found in the selected meetings.",
                question = reduce_question,
            )
        }),
    }
}

// ---------------------------------------------------------------------------
// Pre-call prep prompt (specs/0036) — a second consumer of the same engine.
// ---------------------------------------------------------------------------
//
// Same map-reduce shape as Ask-AI, but there is no question: the task is fixed —
// synthesize a SHORT "what happened last time, what's still open" brief from the
// recent prior occurrences of a recurring meeting. Decisions are emphasized (the
// core "what did we decide" need). Map is still the load-bearing extraction step;
// reduce writes a tight, skimmable brief with [M#] citations to the occurrence.

/// Map stage for prep: per prior-occurrence extraction of decisions, topics, and
/// open threads. Extraction only — no synthesizing the brief here.
const PREP_MAP_SYSTEM: &str = "\
You extract the record of ONE past occurrence of a recurring meeting, to help \
someone prepare for the NEXT occurrence. You are given the meeting's title, date, \
and content.

Your only job is extraction. Do NOT write a brief, do NOT synthesize across \
meetings, do NOT add opinions.

Pull out, verbatim where possible, keeping names, numbers, and dates exact:
- DECISIONS that were made (most important).
- The main topics discussed.
- Open questions, unresolved threads, and commitments/action items still outstanding.

Output a plain list, one item per line, each prefixed with DECISION:, TOPIC:, or \
OPEN: as appropriate. Nothing else. If this occurrence has no substantive content, \
output exactly this single line and nothing more:
NOTHING RELEVANT";

/// Reduce stage for prep: write the short brief with sections + `[M#]` citations.
const PREP_REDUCE_SYSTEM: &str = "\
You write a SHORT pre-meeting brief for the next occurrence of a recurring meeting, \
using extracts from its recent past occurrences. Each occurrence is numbered with a \
marker like [M1], [M2] at the start of its section ([M1] is the most recent).

Write a brief that a busy executive can skim in about twenty seconds, in markdown, \
using ONLY these sections and omitting any that would be empty:
- **Where we left off** — one or two sentences on the current state of this series.
- **Decisions** — bullet points of what was decided (this is the priority).
- **Open threads** — bullet points of unresolved questions or commitments to follow up.

Rules:
- EVERY bullet or claim ends with the marker(s) of the occurrence(s) it came from, \
written back to back for multiple sources. Example: Shipped the pricing page [M1]. \
Hiring backfill raised twice and still open [M1][M2].
- Only use markers that appear in the sources. Never invent one.
- Ignore any source section that says NOTHING RELEVANT.
- Be concrete (names, numbers, dates). Keep the whole brief under ~150 words. Do not \
add a preamble or a closing line — just the sections.
- If the sources contain nothing substantive, reply with a single short sentence \
saying there's no prior history to brief from.";

/// Builds the pre-call-prep prompt pair (specs/0036). No question parameter — the
/// task is fixed. The engine numbers the occurrences; these closures never emit
/// `[M#]` at map stage. Reuse `postprocess_citations` on the reduce output.
pub fn pre_call_prep_prompt() -> AggregationPrompt {
    AggregationPrompt {
        map_system: PREP_MAP_SYSTEM.to_string(),
        map_user: Box::new(|doc: &MeetingDoc| {
            format!(
                "Past occurrence: \"{title}\"\n\
                 Date: {date}\n\
                 Content source: {source}\n\n\
                 Meeting content:\n{content}\n\n\
                 Extract the decisions, topics, and open threads as instructed. If there \
                 is no substantive content, output exactly: NOTHING RELEVANT",
                title = doc.title,
                date = doc.created_at,
                source = source_label(doc.source),
                content = doc.full_text(),
            )
        }),
        reduce_system: PREP_REDUCE_SYSTEM.to_string(),
        reduce_user: Box::new(|block: &str| {
            let filtered = filter_nothing_relevant(block);
            let sources = if filtered.is_empty() {
                EMPTY_SOURCES_NOTICE
            } else {
                filtered.as_str()
            };
            format!(
                "Extracts from the recent past occurrences (most recent first):\n\n{sources}\n\n\
                 Write the brief now, using only these extracts and citing every point with \
                 its [M#] marker. If there is nothing substantive, say there's no prior \
                 history to brief from."
            )
        }),
    }
}

// ---------------------------------------------------------------------------
// Person roll-up prompt (specs/0038 WS5.b) — a third consumer of the same engine.
// ---------------------------------------------------------------------------
//
// Same map-reduce shape as Ask-AI and prep, but the fixed task is scoped to ONE
// PERSON rather than a question or a recurring series: across the person's recent
// meetings, pull what was discussed with / decided about / assigned to them, then
// write a short "recent themes / open threads / last few meetings with {person}"
// brief. Map stays the load-bearing extraction step (name in the user prompt, like
// Ask-AI carries the question there — the system prompt is person-agnostic); reduce
// writes a tight, skimmable briefing with [M#] citations to the meeting.

/// Map stage for the person roll-up: per-meeting extraction of everything about the
/// target person. Extraction only — no synthesizing the roll-up here.
const PERSON_MAP_SYSTEM: &str = "\
You extract, from the record of ONE meeting, everything about a specific PERSON who \
is named in the instruction. You are given the meeting's title, date, and content.

Your only job is extraction. Do NOT write a summary of the whole meeting, do NOT \
synthesize across meetings, do NOT add opinions.

Pull out, verbatim where possible, keeping names, numbers, and dates exact:
- What this person said, asked for, raised, or committed to.
- DECISIONS made about, or directly affecting, this person.
- Action items owned by or assigned to this person, and their status.
- Open questions or unresolved threads involving this person.

Output a plain list, one item per line. Nothing else. If this meeting contains \
nothing about this person, output exactly this single line and nothing more:
NOTHING RELEVANT";

/// Reduce stage for the person roll-up: write the short briefing with sections +
/// `[M#]` citations.
const PERSON_REDUCE_SYSTEM: &str = "\
You write a SHORT roll-up of one person's recent meetings, using extracts from those \
meetings. Each meeting is numbered with a marker like [M1], [M2] at the start of its \
section ([M1] is the most recent).

Write a briefing a busy person can skim in about twenty seconds, in markdown, using \
ONLY these sections and omitting any that would be empty:
- **Recent themes** — one or two bullets on what has been coming up with this person lately.
- **Open threads** — bullets of unresolved questions, commitments, or action items still outstanding.
- **Last few meetings** — one bullet per meeting, most recent first: the gist of what \
involved this person there.

Rules:
- EVERY bullet or claim ends with the marker(s) of the meeting(s) it came from, written \
back to back for multiple sources. Example: Pushed back on the timeline [M1]. Owns the \
pricing doc, still open [M1][M3].
- Only use markers that appear in the sources. Never invent one.
- Ignore any source section that says NOTHING RELEVANT.
- Be concrete (names, numbers, dates). Keep the whole roll-up under ~150 words. Do not \
add a preamble or a closing line — just the sections.
- If the sources contain nothing substantive, reply with a single short sentence saying \
there is nothing recent to roll up for this person.";

/// Builds the person-roll-up prompt pair (specs/0038 WS5.b). The person's display
/// name is threaded through the user prompts (the system prompts stay
/// person-agnostic, mirroring how [`ask_ai_prompt`] keeps its question in the user
/// message). The engine numbers the meetings; these closures never emit `[M#]` at
/// map stage. Reuse `postprocess_citations` on the reduce output.
pub fn person_rollup_prompt(person_name: &str) -> AggregationPrompt {
    let map_name = person_name.trim().to_string();
    let reduce_name = map_name.clone();

    AggregationPrompt {
        map_system: PERSON_MAP_SYSTEM.to_string(),
        map_user: Box::new(move |doc: &MeetingDoc| {
            format!(
                "Person: {person}\n\n\
                 Meeting: \"{title}\"\n\
                 Date: {date}\n\
                 Content source: {source}\n\n\
                 Meeting content:\n{content}\n\n\
                 Extract everything about {person} as instructed. If there is nothing \
                 about them, output exactly: NOTHING RELEVANT",
                person = map_name,
                title = doc.title,
                date = doc.created_at,
                source = source_label(doc.source),
                content = doc.full_text(),
            )
        }),
        reduce_system: PERSON_REDUCE_SYSTEM.to_string(),
        reduce_user: Box::new(move |block: &str| {
            let filtered = filter_nothing_relevant(block);
            let sources = if filtered.is_empty() {
                EMPTY_SOURCES_NOTICE
            } else {
                filtered.as_str()
            };
            format!(
                "Person: {person}\n\n\
                 Extracts from {person}'s recent meetings (most recent first):\n\n{sources}\n\n\
                 Write the roll-up now, using only these extracts and citing every point \
                 with its [M#] marker. If there is nothing substantive, say there is \
                 nothing recent to roll up for {person}.",
                person = reduce_name,
            )
        }),
    }
}

/// Human-readable label for the doc's content source, so the map model knows
/// whether it's reading a distilled summary, the user's own notes, or raw
/// speech (which affects how literally to treat the text).
fn source_label(source: DocSource) -> &'static str {
    match source {
        DocSource::Summary => "AI-generated meeting summary",
        DocSource::Notes => "the user's own notes",
        DocSource::Transcript => "raw meeting transcript",
    }
}

// ---------------------------------------------------------------------------
// NOTHING RELEVANT filtering
// ---------------------------------------------------------------------------

/// Removes sections whose body is the `NOTHING RELEVANT` sentinel from the
/// engine's numbered block before it reaches the reduce model.
///
/// The engine formats both single-pass doc blocks and map outputs as sections
/// headed by `[M#] "title" (date)` lines, so filtering on that header shape is
/// deterministic. Only bodies that are *exactly* the sentinel (optionally with
/// a trailing period) are dropped — near-misses ("Nothing relevant was said…")
/// are left for the reduce prompt's ignore instruction, which is safer than a
/// fuzzy match eating real content. Dropped meetings keep their number: the
/// remaining headers are engine-assigned and are not renumbered, so `[M#]`
/// citations still resolve to `sources[#-1]`.
fn filter_nothing_relevant(block: &str) -> String {
    let mut sections: Vec<Vec<&str>> = Vec::new();
    for line in block.lines() {
        if is_marker_header(line) || sections.is_empty() {
            sections.push(vec![line]);
        } else {
            // `unwrap` is safe: the branch above guarantees non-empty.
            sections.last_mut().unwrap().push(line);
        }
    }

    sections
        .into_iter()
        .filter(|section| !is_nothing_relevant_section(section))
        .map(|section| section.join("\n").trim_end().to_string())
        .filter(|section| !section.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// True when the section has an engine header and its body is exactly the
/// `NOTHING RELEVANT` sentinel.
fn is_nothing_relevant_section(section: &[&str]) -> bool {
    if section.is_empty() || !is_marker_header(section[0]) {
        return false;
    }
    let body = section[1..].join("\n");
    let body = body.trim();
    body.eq_ignore_ascii_case("NOTHING RELEVANT") || body.eq_ignore_ascii_case("NOTHING RELEVANT.")
}

/// Matches the engine's section header shape (the parser side of
/// [`crate::aggregation::engine::numbered_section`], the wire format's single
/// definition): `[M<digits>] "` — the trailing quote (start of the title)
/// keeps model-emitted `[M1]` citations inside a body from being mistaken for
/// a new section.
fn is_marker_header(line: &str) -> bool {
    let Some(rest) = line.strip_prefix("[M") else {
        return false;
    };
    let digits = rest.chars().take_while(|c| c.is_ascii_digit()).count();
    if digits == 0 {
        return false;
    }
    rest[digits..].starts_with("] \"")
}

// ---------------------------------------------------------------------------
// Citation post-processing
// ---------------------------------------------------------------------------

/// Validates and normalizes `[M#]` citation markers in the reduce output.
///
/// - Out-of-range markers (e.g. `[M7]` with 3 sources) are stripped from the
///   text (with a little whitespace tidying around the removal).
/// - Variant forms are normalized to canonical back-to-back markers:
///   `[m1]` → `[M1]`, `[M1, M2]` / `[M1 M2]` → `[M1][M2]` (in-group duplicates
///   collapse).
/// - Markdown-link-shaped citations (`[M1](anything)`) are normalized to the
///   bare chip marker, dropping the parenthetical: the model has no real URLs
///   in its context, so any link target is hallucinated — and the frontend
///   renders a chip for every in-range `[M#]`, which would leave a dangling
///   `(url)`. Out-of-range link markers are stripped entirely, parenthetical
///   included.
/// - `sources[i].cited` is set to `true` exactly when `[M{i+1}]` is referenced
///   after normalization/stripping (pre-existing flags are reset).
///
/// Conservative by design: a bracket group is touched only when its entire
/// content is a list of `M<number>` refs. `[TODO]`, `[1]`, `[M1 and M2]`,
/// nested brackets, and genuine links like `[docs](https://…)` pass through
/// verbatim.
pub fn postprocess_citations(markdown: &str, sources: &mut [SourceMeeting]) -> String {
    for source in sources.iter_mut() {
        source.cited = false;
    }
    let source_count = sources.len();

    let mut out = String::with_capacity(markdown.len());
    let mut i = 0; // byte index, always on a char boundary

    while i < markdown.len() {
        let rest = &markdown[i..];
        if rest.starts_with('[') {
            // Candidate group: up to the first ']', with no nested '['.
            if let Some(close) = rest.find(']') {
                let inner = &rest[1..close];
                if !inner.contains('[') {
                    if let Some(refs) = parse_citation_group(inner) {
                        let mut next = i + close + 1;
                        // Link-shaped citation `[M#](…)`: consume the
                        // parenthetical too, so the marker normalizes to the
                        // bare chip form (or strips wholesale when out of
                        // range). An unclosed `(` is left in the text.
                        if rest[close + 1..].starts_with('(') {
                            if let Some(paren_close) = rest[close + 2..].find(')') {
                                next = i + close + 2 + paren_close + 1;
                            }
                        }
                        let in_range: Vec<usize> = refs
                            .into_iter()
                            .filter(|&r| r >= 1 && r <= source_count)
                            .fold(Vec::new(), |mut acc, r| {
                                if !acc.contains(&r) {
                                    acc.push(r);
                                }
                                acc
                            });

                        if in_range.is_empty() {
                            // Whole group stripped: tidy the surrounding
                            // whitespace so "claim [M7]." → "claim." and
                            // "claim [M7] end" → "claim end".
                            if out.ends_with(' ') {
                                out.pop();
                            } else if (out.is_empty() || out.ends_with('\n'))
                                && markdown[next..].starts_with(' ')
                            {
                                next += 1;
                            }
                        } else {
                            for r in in_range {
                                sources[r - 1].cited = true;
                                out.push_str(&format!("[M{r}]"));
                            }
                        }
                        i = next;
                        continue;
                    }
                }
            }
        }

        // Not a citation group: copy one char and keep scanning (a nested '['
        // is re-examined on its own turn).
        let ch = rest.chars().next().expect("non-empty remainder");
        out.push(ch);
        i += ch.len_utf8();
    }

    out
}

/// Parses a bracket-group interior as a citation list: refs separated by
/// commas and/or whitespace, each `M`/`m` followed by digits. Returns `None`
/// (leave the text alone) when *any* token doesn't fit that shape.
fn parse_citation_group(inner: &str) -> Option<Vec<usize>> {
    let mut refs = Vec::new();
    for token in inner.split(|c: char| c == ',' || c.is_whitespace()) {
        if token.is_empty() {
            continue; // artifacts of ", " / padded splits
        }
        let digits = token.strip_prefix(['M', 'm'])?;
        if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
            return None;
        }
        refs.push(digits.parse::<usize>().ok()?);
    }
    if refs.is_empty() {
        None // "[]", "[, ]" — not citations
    } else {
        Some(refs)
    }
}

// ---------------------------------------------------------------------------
// Tests: citation mechanics + prompt-eval fixtures over the fake LLM
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::aggregation::engine::{run, Stage};
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::{Arc, Mutex};
    use tokio_util::sync::CancellationToken;

    fn sources(n: usize) -> Vec<SourceMeeting> {
        (1..=n)
            .map(|i| SourceMeeting {
                meeting_id: format!("m{i}"),
                title: format!("Meeting {i}"),
                created_at: format!("2026-06-{i:02}T10:00:00Z"),
                cited: false,
            })
            .collect()
    }

    fn cited_flags(sources: &[SourceMeeting]) -> Vec<bool> {
        sources.iter().map(|s| s.cited).collect()
    }

    // -- postprocess_citations ------------------------------------------------

    #[test]
    fn normalizes_lowercase_and_comma_separated_groups() {
        let mut srcs = sources(3);
        let out = postprocess_citations(
            "Launch moved to March [m1]. Pricing raised twice [M1, M3]. Also [M2 M3].",
            &mut srcs,
        );
        assert_eq!(
            out,
            "Launch moved to March [M1]. Pricing raised twice [M1][M3]. Also [M2][M3]."
        );
        assert_eq!(cited_flags(&srcs), vec![true, true, true]);
    }

    #[test]
    fn strips_out_of_range_markers() {
        let mut srcs = sources(3);
        let out = postprocess_citations("The budget was cut [M7]. Confirmed [M2].", &mut srcs);
        assert_eq!(out, "The budget was cut. Confirmed [M2].");
        assert_eq!(cited_flags(&srcs), vec![false, true, false]);
    }

    #[test]
    fn mixed_group_keeps_in_range_refs_only() {
        let mut srcs = sources(3);
        let out = postprocess_citations("Decided in two meetings [M1, M7].", &mut srcs);
        assert_eq!(out, "Decided in two meetings [M1].");
        assert_eq!(cited_flags(&srcs), vec![true, false, false]);
    }

    #[test]
    fn fully_stripped_group_tidies_whitespace() {
        let mut srcs = sources(1);
        assert_eq!(
            postprocess_citations("claim [M7] end", &mut srcs),
            "claim end"
        );
        assert_eq!(
            postprocess_citations("- [M9] lead bullet", &mut srcs),
            "- lead bullet"
        );
        assert_eq!(
            postprocess_citations("[M0] starts line", &mut srcs),
            "starts line"
        );
        assert_eq!(cited_flags(&srcs), vec![false]);
    }

    #[test]
    fn cited_flags_are_exact_and_reset() {
        // Pre-set flags must be recomputed from scratch.
        let mut srcs = sources(3);
        srcs[0].cited = true;
        srcs[2].cited = true;
        let out = postprocess_citations("Only the retro covered this [M2].", &mut srcs);
        assert_eq!(out, "Only the retro covered this [M2].");
        assert_eq!(cited_flags(&srcs), vec![false, true, false]);
    }

    #[test]
    fn double_digit_markers_do_not_collide_with_single_digit() {
        let mut srcs = sources(12);
        let out = postprocess_citations("See [M10] and [M12], not [M1].", &mut srcs);
        assert_eq!(out, "See [M10] and [M12], not [M1].");
        assert!(srcs[0].cited);
        assert!(srcs[9].cited);
        assert!(srcs[11].cited);
        assert!(!srcs[1].cited);
    }

    #[test]
    fn non_citation_brackets_pass_through_verbatim() {
        let mut srcs = sources(2);
        let text = "[TODO] check [1] and [see M1 notes] and [M1x] and [] and [M1 and M2]";
        assert_eq!(postprocess_citations(text, &mut srcs), text);
        assert_eq!(cited_flags(&srcs), vec![false, false]);
    }

    #[test]
    fn in_range_citation_links_normalize_to_bare_chip_markers() {
        let mut srcs = sources(2);
        let text = "See [M1](https://example.com/notes) for details [M2].";
        assert_eq!(
            postprocess_citations(text, &mut srcs),
            "See [M1] for details [M2]."
        );
        assert_eq!(cited_flags(&srcs), vec![true, true]);

        // Group form + lowercase inside a link normalizes the same way.
        let mut srcs = sources(2);
        assert_eq!(
            postprocess_citations("agreed [m1, M2](notes.md)", &mut srcs),
            "agreed [M1][M2]"
        );
        assert_eq!(cited_flags(&srcs), vec![true, true]);
    }

    #[test]
    fn out_of_range_citation_links_are_stripped_with_their_parenthetical() {
        let mut srcs = sources(1);
        assert_eq!(
            postprocess_citations("claim [M7](https://example.com) end", &mut srcs),
            "claim end"
        );
        assert_eq!(
            postprocess_citations("[M9](x) starts line", &mut srcs),
            "starts line"
        );
        assert_eq!(cited_flags(&srcs), vec![false]);
    }

    #[test]
    fn genuine_non_citation_links_are_left_alone() {
        let mut srcs = sources(2);
        let text = "Read [docs](https://example.com) and [see M1 notes](notes.md).";
        assert_eq!(postprocess_citations(text, &mut srcs), text);
        assert_eq!(cited_flags(&srcs), vec![false, false]);

        // An unclosed parenthetical keeps its text; the marker still counts.
        assert_eq!(
            postprocess_citations("dangling [M1](oops", &mut srcs),
            "dangling [M1](oops"
        );
        assert_eq!(cited_flags(&srcs), vec![true, false]);
    }

    #[test]
    fn nested_and_unclosed_brackets_are_preserved() {
        let mut srcs = sources(2);
        assert_eq!(
            postprocess_citations("weird [a [M1] b] and dangling [M2", &mut srcs),
            "weird [a [M1] b] and dangling [M2"
        );
        // The inner [M1] IS a well-formed citation and is counted.
        assert_eq!(cited_flags(&srcs), vec![true, false]);
    }

    #[test]
    fn duplicate_refs_in_one_group_collapse() {
        let mut srcs = sources(2);
        assert_eq!(
            postprocess_citations("agreed [M1, M1, m1]", &mut srcs),
            "agreed [M1]"
        );
        assert_eq!(cited_flags(&srcs), vec![true, false]);
    }

    #[test]
    fn no_sources_strips_everything() {
        let mut srcs = sources(0);
        assert_eq!(postprocess_citations("claim [M1].", &mut srcs), "claim.");
    }

    #[test]
    fn handles_multibyte_text_around_markers() {
        let mut srcs = sources(1);
        let out = postprocess_citations("décidé — voir [m1] ✓ [M4]", &mut srcs);
        assert_eq!(out, "décidé — voir [M1] ✓");
        assert_eq!(cited_flags(&srcs), vec![true]);
    }

    // -- NOTHING RELEVANT filtering -------------------------------------------

    fn engine_block(sections: &[(usize, &str, &str)]) -> String {
        // The REAL engine section constructor (`n` is the 1-based marker
        // number), so these fixtures can't drift from the wire format.
        sections
            .iter()
            .map(|(n, title, body)| {
                crate::aggregation::engine::numbered_section(n - 1, title, "2026-06-01", body)
            })
            .collect::<Vec<_>>()
            .join("\n\n")
    }

    #[test]
    fn filter_drops_nothing_relevant_sections_and_keeps_numbering() {
        let block = engine_block(&[
            (1, "Kickoff", "- \"we ship in March\" (Ana)"),
            (2, "Standup", "NOTHING RELEVANT"),
            (
                3,
                "Retro",
                "- pricing raised to $12\n- follow-up owned by Ben",
            ),
        ]);
        let filtered = filter_nothing_relevant(&block);
        assert!(filtered.contains("[M1] \"Kickoff\""));
        assert!(!filtered.contains("[M2]"));
        assert!(!filtered.contains("NOTHING RELEVANT"));
        // M3 keeps its engine-assigned number — no renumbering.
        assert!(filtered.contains("[M3] \"Retro\""));
        assert!(filtered.contains("owned by Ben"));
    }

    #[test]
    fn filter_accepts_sentinel_variants_case_and_period() {
        let block = engine_block(&[(1, "Standup", "nothing relevant."), (2, "Retro", "real")]);
        let filtered = filter_nothing_relevant(&block);
        assert!(!filtered.contains("[M1]"));
        assert!(filtered.contains("[M2] \"Retro\""));
    }

    #[test]
    fn filter_keeps_near_miss_bodies_for_the_model_to_ignore() {
        // Fuzzy variants are NOT eaten — the reduce prompt handles them.
        let block = engine_block(&[(1, "Standup", "Nothing relevant was discussed here.")]);
        assert_eq!(filter_nothing_relevant(&block), block);
    }

    #[test]
    fn filter_does_not_split_on_citation_markers_inside_bodies() {
        // A body line mentioning [M1] (no `"` header shape) must not start a
        // new section.
        let block = engine_block(&[(
            1,
            "Kickoff",
            "- see also [M2] for pricing\nNOTHING RELEVANT",
        )]);
        // Body is NOT exactly the sentinel (it has an extra line) → kept whole.
        assert_eq!(filter_nothing_relevant(&block), block);
    }

    // -- prompt formation ------------------------------------------------------

    fn doc(id: &str, title: &str, text: &str) -> MeetingDoc {
        MeetingDoc {
            meeting_id: id.to_string(),
            title: title.to_string(),
            created_at: "2026-06-15T10:00:00Z".to_string(),
            source: DocSource::Summary,
            text: text.to_string(),
            excerpt: None,
        }
    }

    #[test]
    fn map_user_prompt_carries_question_metadata_and_content() {
        let prompt = ask_ai_prompt("  what did we decide about pricing?  ");
        let mut d = doc("m1", "Kickoff", "Pricing goes to $12.");
        d.source = DocSource::Notes;
        d.excerpt = Some("Ana: twelve dollars, final.".to_string());

        let user = (prompt.map_user)(&d);
        assert!(user.contains("Question: what did we decide about pricing?"));
        assert!(user.contains("Meeting: \"Kickoff\""));
        assert!(user.contains("Date: 2026-06-15T10:00:00Z"));
        assert!(user.contains("Content source: the user's own notes"));
        assert!(user.contains("Pricing goes to $12."));
        // full_text() folds the transcript-evidence excerpt in.
        assert!(user.contains("Ana: twelve dollars, final."));
        assert!(user.contains("NOTHING RELEVANT"));
        // Map stage never numbers anything itself.
        assert!(!user.contains("[M1]"));
    }

    #[test]
    fn reduce_system_states_the_citation_contract() {
        let prompt = ask_ai_prompt("q");
        assert!(
            prompt.reduce_system.contains("[M1][M3]"),
            "needs a back-to-back example"
        );
        assert!(prompt.reduce_system.contains("Never invent a marker"));
        assert!(prompt.reduce_system.contains("NOTHING RELEVANT"));
        assert!(prompt
            .reduce_system
            .contains("not found in the selected meetings"));
    }

    #[test]
    fn reduce_user_filters_sentinels_and_repeats_the_question() {
        let prompt = ask_ai_prompt("what did we decide?");
        let block = engine_block(&[
            (1, "Kickoff", "- \"we ship in March\""),
            (2, "Standup", "NOTHING RELEVANT"),
        ]);
        let user = (prompt.reduce_user)(&block);
        assert!(user.contains("Question: what did we decide?"));
        assert!(user.contains("[M1] \"Kickoff\""));
        assert!(!user.contains("[M2]"));
        assert!(!user.contains("NOTHING RELEVANT"));
    }

    #[test]
    fn reduce_user_stays_well_formed_when_all_maps_were_irrelevant() {
        // The not-found path: every map said NOTHING RELEVANT.
        let prompt = ask_ai_prompt("what about llamas?");
        let block = engine_block(&[
            (1, "Kickoff", "NOTHING RELEVANT"),
            (2, "Retro", "NOTHING RELEVANT"),
        ]);
        let user = (prompt.reduce_user)(&block);
        assert!(user.contains("Question: what about llamas?"));
        assert!(user.contains(EMPTY_SOURCES_NOTICE));
        assert!(!user.contains("NOTHING RELEVANT"));
        assert!(user.contains("not found in the selected meetings"));
    }

    // -- person roll-up prompt (specs/0038 WS5.b) ------------------------------

    #[test]
    fn person_map_user_carries_name_metadata_and_content() {
        let prompt = person_rollup_prompt("  Ana Ruiz  ");
        let mut d = doc("m1", "Kickoff", "Ana owns the pricing doc.");
        d.source = DocSource::Notes;

        let user = (prompt.map_user)(&d);
        assert!(user.contains("Person: Ana Ruiz")); // trimmed
        assert!(user.contains("Meeting: \"Kickoff\""));
        assert!(user.contains("Date: 2026-06-15T10:00:00Z"));
        assert!(user.contains("Content source: the user's own notes"));
        assert!(user.contains("Ana owns the pricing doc."));
        assert!(user.contains("NOTHING RELEVANT"));
        // Map stage never numbers anything itself.
        assert!(!user.contains("[M1]"));
    }

    #[test]
    fn person_reduce_system_states_the_citation_contract_and_sections() {
        let prompt = person_rollup_prompt("Ana");
        assert!(
            prompt.reduce_system.contains("[M1][M3]"),
            "needs a back-to-back example"
        );
        assert!(prompt.reduce_system.contains("Never invent one"));
        assert!(prompt.reduce_system.contains("NOTHING RELEVANT"));
        assert!(prompt.reduce_system.contains("Recent themes"));
        assert!(prompt.reduce_system.contains("Open threads"));
        assert!(prompt.reduce_system.contains("Last few meetings"));
    }

    #[test]
    fn person_reduce_user_filters_sentinels_and_names_the_person() {
        let prompt = person_rollup_prompt("Ana");
        let block = engine_block(&[
            (1, "Kickoff", "Ana owns pricing"),
            (2, "Standup", "NOTHING RELEVANT"),
        ]);
        let user = (prompt.reduce_user)(&block);
        assert!(user.contains("Person: Ana"));
        assert!(user.contains("[M1] \"Kickoff\""));
        assert!(!user.contains("[M2]"));
        assert!(!user.contains("NOTHING RELEVANT"));
    }

    // -- end-to-end through engine::run with a fake LLM ------------------------

    type LlmFuture = Pin<Box<dyn Future<Output = Result<String, String>>>>;

    /// A budget that forces map-reduce (two docs can't share one pass) while
    /// letting every map call fit without chunking. Derived from the real
    /// prompt sizes so wording changes don't silently flip the tests' mode —
    /// the calls-count assertions would catch it anyway, loudly.
    fn map_reduce_budget(prompt: &AggregationPrompt, docs: &[MeetingDoc]) -> usize {
        use crate::summary::processor::rough_token_count;
        let max_map = docs
            .iter()
            .map(|d| rough_token_count(&(prompt.map_user)(d)))
            .max()
            .expect("at least one doc");
        // 400 > the engine's 300-token overhead reserve, with headroom.
        rough_token_count(&prompt.map_system) + max_map + 400
    }

    /// Fake LLM for prompt-eval fixtures: routes map calls by meeting title,
    /// reduce calls to a canned answer, and records every user prompt.
    struct ScriptedLlm {
        map_answers: Vec<(&'static str, &'static str)>, // (title substring, output)
        reduce_answer: &'static str,
        calls: Arc<Mutex<Vec<(String, String)>>>,
    }

    impl ScriptedLlm {
        fn closure(&self) -> impl Fn(String, String) -> LlmFuture {
            let map_answers = self.map_answers.clone();
            let reduce_answer = self.reduce_answer.to_string();
            let calls = self.calls.clone();
            move |system: String, user: String| {
                let map_answers = map_answers.clone();
                let reduce_answer = reduce_answer.clone();
                let calls = calls.clone();
                Box::pin(async move {
                    calls.lock().unwrap().push((system.clone(), user.clone()));
                    if user.contains("Numbered meeting sources:") {
                        Ok(reduce_answer)
                    } else {
                        let (_, out) = map_answers
                            .iter()
                            .find(|(title, _)| user.contains(title))
                            .expect("map call for an unexpected meeting");
                        Ok(out.to_string())
                    }
                })
            }
        }
    }

    #[tokio::test]
    async fn e2e_map_reduce_filters_irrelevant_meeting_and_cites_correctly() {
        let fake = ScriptedLlm {
            map_answers: vec![
                ("Kickoff", "- \"pricing goes to $12 a seat\" (Ana)"),
                ("Standup", "NOTHING RELEVANT"),
            ],
            reduce_answer: "Pricing was set at $12 a seat [m1]. Nothing else was decided [M1, M9].",
            calls: Arc::new(Mutex::new(Vec::new())),
        };
        let prompt = ask_ai_prompt("what did we decide about pricing?");
        // Bodies sized so two docs can't share one pass but each fits a map call.
        let filler = "word ".repeat(200);
        let docs = vec![
            doc(
                "m1",
                "Kickoff",
                &format!("Pricing goes to $12 a seat. {filler}"),
            ),
            doc("m2", "Standup", &format!("Bug triage only. {filler}")),
        ];
        let budget = map_reduce_budget(&prompt, &docs);
        let cancel = CancellationToken::new();
        let stages = Arc::new(Mutex::new(Vec::new()));
        let sink = stages.clone();

        let mut answer = run(
            fake.closure(),
            &prompt,
            &docs,
            budget,
            &cancel,
            move |s: Stage| sink.lock().unwrap().push(s),
        )
        .await
        .unwrap();

        // The reduce call saw M1 but not the filtered M2 section.
        let calls = fake.calls.lock().unwrap().clone();
        assert_eq!(calls.len(), 3, "two maps + one reduce");
        let reduce_user = &calls[2].1;
        assert!(reduce_user.contains("[M1] \"Kickoff\""));
        assert!(!reduce_user.contains("[M2]"));
        assert!(!reduce_user.contains("NOTHING RELEVANT"));

        // Task-4 post-processing over the engine's raw answer.
        answer.markdown = postprocess_citations(&answer.markdown, &mut answer.sources);
        assert_eq!(
            answer.markdown,
            "Pricing was set at $12 a seat [M1]. Nothing else was decided [M1]."
        );
        assert!(answer.sources[0].cited);
        assert!(
            !answer.sources[1].cited,
            "filtered meeting must end uncited"
        );
    }

    #[tokio::test]
    async fn e2e_single_pass_normalizes_variant_citations() {
        let fake = ScriptedLlm {
            map_answers: vec![],
            reduce_answer: "Launch is in March [m1]. Both meetings confirmed it [M1, M2] [M7].",
            calls: Arc::new(Mutex::new(Vec::new())),
        };
        let prompt = ask_ai_prompt("when do we launch?");
        let docs = vec![
            doc("m1", "Kickoff", "Launch in March."),
            doc("m2", "Retro", "March confirmed."),
        ];
        let cancel = CancellationToken::new();

        let mut answer = run(fake.closure(), &prompt, &docs, 100_000, &cancel, |_| {})
            .await
            .unwrap();

        assert_eq!(
            fake.calls.lock().unwrap().len(),
            1,
            "generous budget → single reduce call, no maps"
        );

        answer.markdown = postprocess_citations(&answer.markdown, &mut answer.sources);
        assert_eq!(
            answer.markdown,
            "Launch is in March [M1]. Both meetings confirmed it [M1][M2]."
        );
        assert!(answer.sources[0].cited);
        assert!(answer.sources[1].cited);
    }

    #[tokio::test]
    async fn e2e_not_found_path_forms_a_sensible_reduce_prompt() {
        let fake = ScriptedLlm {
            map_answers: vec![
                ("Kickoff", "NOTHING RELEVANT"),
                ("Retro", "NOTHING RELEVANT"),
            ],
            reduce_answer: "The answer was not found in the selected meetings.",
            calls: Arc::new(Mutex::new(Vec::new())),
        };
        let prompt = ask_ai_prompt("what about llamas?");
        let filler = "word ".repeat(200);
        let docs = vec![doc("m1", "Kickoff", &filler), doc("m2", "Retro", &filler)];
        let budget = map_reduce_budget(&prompt, &docs);
        let cancel = CancellationToken::new();

        let mut answer = run(fake.closure(), &prompt, &docs, budget, &cancel, |_| {})
            .await
            .unwrap();

        let calls = fake.calls.lock().unwrap().clone();
        let reduce_user = &calls.last().unwrap().1;
        assert!(reduce_user.contains(EMPTY_SOURCES_NOTICE));
        assert!(reduce_user.contains("Question: what about llamas?"));

        answer.markdown = postprocess_citations(&answer.markdown, &mut answer.sources);
        assert_eq!(
            answer.markdown,
            "The answer was not found in the selected meetings."
        );
        assert!(answer.sources.iter().all(|s| !s.cited));
    }
}
