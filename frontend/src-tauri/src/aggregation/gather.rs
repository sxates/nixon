//! Gather stage: turn a question + scope into per-meeting docs (specs/0035).
//!
//! Read-only over existing tables; zero LLM calls, zero egress. Selection is
//! delegated to `SearchRepository::rank_meetings_for_question` (recall-oriented
//! FTS + metadata scope); this module then builds one [`MeetingDoc`] per
//! selected meeting under the **summaries-first source policy**: the summary is
//! the distilled record (~10–50x cheaper in tokens than the transcript), notes
//! are the fallback, and the raw transcript is used only when neither exists.
//!
//! **Transcript-excerpt policy (specs/0056 W3):** whenever the question hit the
//! transcript at all and the body is not already the transcript, the
//! speaker-labelled windows around the meeting's top transcript hits are
//! appended as evidence. This is deliberately independent of which source
//! ranked best: bm25 scores are not comparable across the three FTS tables,
//! so a short, dense summary routinely out-ranks one transcript segment on
//! generic question terms even when only the transcript carries the answer.

use crate::aggregation::scope::AggregationScope;
use crate::database::repositories::meeting_note::MeetingNotesRepository;
use crate::database::repositories::search::SearchRepository;
use crate::database::repositories::transcript::TranscriptsRepository;
use anyhow::Context;
use sqlx::SqlitePool;
use tracing::info;

/// Default cap on meetings per aggregation run (specs/0035: decided v1 cap).
pub const DEFAULT_MAX_MEETINGS: usize = 10;

/// Segments included on each side of each transcript hit when appending a
/// bounded excerpt (specs/0035: ±N segments, default 15). Overlapping windows
/// merge, so K hits cost at most K × (2N + 1) segments.
pub const TRANSCRIPT_EXCERPT_WINDOW: usize = 15;

/// Which content the doc body came from, per the source-selection policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocSource {
    /// `summary_processes.summary_text` (display markdown only — never
    /// `english_cache`/`result_backup`; the generated column enforces this).
    Summary,
    /// `meeting_notes.notes_markdown` (+ `enhanced_markdown` when present).
    Notes,
    /// `TranscriptsRepository::get_full_transcript` — only when neither summary
    /// nor notes exist (e.g. recorded but never summarized).
    Transcript,
}

/// One meeting's contribution to an aggregation run.
#[derive(Debug, Clone)]
pub struct MeetingDoc {
    pub meeting_id: String,
    pub title: String,
    pub created_at: String,
    pub source: DocSource,
    /// Doc body, per the source-selection policy above.
    pub text: String,
    /// Speaker-labelled transcript windows around the question's transcript
    /// hits — present whenever the transcript had a hit and the body is
    /// summary/notes (see the module doc's transcript-excerpt policy).
    pub excerpt: Option<String>,
}

impl MeetingDoc {
    /// The body plus the transcript-evidence excerpt (when present) — what the
    /// engine counts tokens over and packs into prompts.
    pub fn full_text(&self) -> String {
        match self.excerpt.as_deref() {
            Some(excerpt) => format!(
                "{}\n\nTranscript excerpt (evidence for the question):\n{}",
                self.text, excerpt
            ),
            None => self.text.clone(),
        }
    }
}

/// Result of the gather stage.
#[derive(Debug, Clone)]
pub struct GatherResult {
    /// Ranked (best-first) and capped at the caller's `max_meetings`.
    pub docs: Vec<MeetingDoc>,
    /// Meetings that matched but fell beyond the cap — surfaced in the egress
    /// preview as "top N of N+dropped".
    pub dropped: usize,
}

/// Gathers the meeting docs an aggregation question should run over.
///
/// Ranks meetings via the recall-oriented FTS query (or metadata-only fallback
/// for explicit ids / term-less questions), then builds each doc under the
/// summaries-first policy. Meetings with no content at all (no summary, no
/// notes, empty transcript) are skipped — they would contribute nothing to the
/// model and shouldn't count against the egress preview.
pub async fn gather(
    pool: &SqlitePool,
    question: &str,
    scope: &AggregationScope,
    max_meetings: usize,
) -> anyhow::Result<GatherResult> {
    let ranking =
        SearchRepository::rank_meetings_for_question(pool, question, scope, max_meetings)
            .await
            .context("Failed to rank meetings for the question — the search index may need a rebuild (restart the app to re-run migrations)")?;

    let mut docs = Vec::with_capacity(ranking.meetings.len());

    for meeting in &ranking.meetings {
        let (source, text) = match select_doc_body(pool, &meeting.meeting_id).await? {
            Some(body) => body,
            None => {
                info!(
                    "gather: meeting {} has no summary, notes, or transcript — skipping",
                    meeting.meeting_id
                );
                continue;
            }
        };

        // Transcript-excerpt policy (specs/0056 W3, see the module doc): when
        // the question hit the transcript but the doc body is the summary/notes,
        // append the speaker-labelled windows around the top transcript hits so
        // the answer can quote what was actually said, and by whom. Whether the
        // summary or the transcript ranked best is irrelevant here — bm25 is
        // not comparable across FTS tables. When the body already IS the
        // transcript, the evidence is included by construction.
        let excerpt = if !meeting.transcript_hit_ids.is_empty() && source != DocSource::Transcript {
            TranscriptsRepository::get_transcript_excerpts_with_speakers(
                pool,
                &meeting.meeting_id,
                &meeting.transcript_hit_ids,
                TRANSCRIPT_EXCERPT_WINDOW,
            )
            .await
            .with_context(|| {
                format!(
                    "Failed to load the transcript excerpt for meeting {}",
                    meeting.meeting_id
                )
            })?
            .filter(|excerpt| !excerpt.is_empty())
        } else {
            None
        };

        docs.push(MeetingDoc {
            meeting_id: meeting.meeting_id.clone(),
            title: meeting.title.clone(),
            created_at: meeting.created_at.clone(),
            source,
            text,
            excerpt,
        });
    }

    info!(
        "gather: {} doc(s) built ({} matched beyond the cap)",
        docs.len(),
        ranking.dropped
    );

    Ok(GatherResult {
        docs,
        dropped: ranking.dropped,
    })
}

/// Applies the source-selection policy for one meeting:
/// summary → notes (+ enhanced) → raw transcript → `None` (no content).
async fn select_doc_body(
    pool: &SqlitePool,
    meeting_id: &str,
) -> anyhow::Result<Option<(DocSource, String)>> {
    // 1. Summary display markdown via the 0033 generated column ($.markdown only).
    let summary: Option<String> =
        sqlx::query_scalar("SELECT summary_text FROM summary_processes WHERE meeting_id = ?")
            .bind(meeting_id)
            .fetch_optional(pool)
            .await
            .with_context(|| format!("Failed to load the summary for meeting {meeting_id}"))?
            .flatten();

    if let Some(summary) = summary
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
    {
        return Ok(Some((DocSource::Summary, summary)));
    }

    // 2. User notes (+ the enhanced twin when present).
    let notes = MeetingNotesRepository::get_notes(pool, meeting_id)
        .await
        .with_context(|| format!("Failed to load the notes for meeting {meeting_id}"))?;
    if let Some(note) = notes {
        let mut parts: Vec<String> = Vec::new();
        if let Some(markdown) = note
            .notes_markdown
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            parts.push(markdown.to_string());
        }
        if let Some(enhanced) = note
            .enhanced_markdown
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            parts.push(enhanced.to_string());
        }
        if !parts.is_empty() {
            return Ok(Some((DocSource::Notes, parts.join("\n\n"))));
        }
    }

    // 3. Raw transcript — only when neither summary nor notes exist.
    let transcript = TranscriptsRepository::get_full_transcript(pool, meeting_id)
        .await
        .with_context(|| format!("Failed to load the transcript for meeting {meeting_id}"))?;
    if transcript.trim().is_empty() {
        Ok(None)
    } else {
        Ok(Some((DocSource::Transcript, transcript)))
    }
}
