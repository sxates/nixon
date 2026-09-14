//! Aggregation engine + Ask-AI integration tests (specs/0035, task 6).
//!
//! Exercises the whole pipeline — gather (FTS5 + scope filters, summaries-first
//! source policy), budget packing, map-reduce, citations, token-estimate
//! parity, and cancellation — over the app's REAL migration path
//! (`DatabaseManager::new` runs every file in `migrations/`, including the
//! 0033 FTS triggers, so plain repository writes index themselves). NO network,
//! NO Tauri runtime: every LLM is an injected fake closure, per the engine's
//! consumer contract.
//!
//! Fixtures (a)–(f) from the spec (the (e) preview command was removed
//! 2026-07-04 with the pre-send consent gate; its parity/cap assertions now
//! target gather + `estimate_run_tokens` directly):
//!   (a) two summary meetings each holding half the answer -> both gathered,
//!       fake-LLM run cites [M1] and [M2]
//!   (b) person + date scope filters exclude out-of-scope meetings
//!   (c) summaries-first: transcript body NOT included when a summary exists;
//!       bounded excerpt appended whenever the question hit the transcript
//!       (specs/0056 W3: regardless of which source ranked best — bm25 is not
//!       comparable across the three FTS tables), speaker-labelled
//!   (d) no-summary meeting falls back to notes, then transcript
//!   (e) estimate parity (estimate covers what a run actually sends) + the
//!       over-cap dropped count
//!   (f) cancellation mid-map stops promptly, no reduce call
//! Plus: the engine consumed with a NON-Ask prompt (consumer-agnostic contract).
//!
//! Run with:
//!   cargo test --features metal --test aggregation_engine -- --nocapture

mod common;

use app_lib::aggregation::{
    ask_ai_prompt, estimate_run_tokens, execute_ask_ai, gather, run, AggregationPrompt,
    AggregationScope, DocSource, MeetingDoc, Stage, DEFAULT_MAX_MEETINGS,
};
use app_lib::database::repositories::meeting::MeetingsRepository;
use app_lib::database::repositories::meeting_note::MeetingNotesRepository;
use app_lib::database::repositories::search::SearchRepository;
use app_lib::database::repositories::speaker::SpeakersRepository;
use app_lib::database::repositories::summary::SummaryProcessesRepository;
use app_lib::database::repositories::transcript::TranscriptsRepository;
use app_lib::summary::rough_token_count;
use app_lib::transcripts::TranscriptSegment;
use common::{fresh_db, segment};
use serde_json::json;
use sqlx::SqlitePool;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use tokio_util::sync::CancellationToken;

// ---------------------------------------------------------------------------
// Fixture helpers
// ---------------------------------------------------------------------------

/// Create a meeting with a title and a deterministic `created_at`.
async fn create_meeting(pool: &SqlitePool, title: &str, created_at: &str) -> String {
    let id = MeetingsRepository::create_meeting(pool, Some(title.into()), None, None, None, None)
        .await
        .expect("create_meeting");
    sqlx::query("UPDATE meetings SET created_at = ? WHERE id = ?")
        .bind(created_at)
        .bind(&id)
        .execute(pool)
        .await
        .expect("set created_at");
    id
}

/// Attach transcript segments through the real save path (fires FTS triggers).
async fn add_transcript(pool: &SqlitePool, meeting_id: &str, title: &str, texts: &[&str]) {
    let segments: Vec<TranscriptSegment> = texts
        .iter()
        .enumerate()
        .map(|(i, t)| segment(t, i as f64 * 2.0, i as f64 * 2.0 + 2.0))
        .collect();
    let attached = TranscriptsRepository::save_transcripts_for_meeting(
        pool, meeting_id, title, &segments, None,
    )
    .await
    .expect("save_transcripts_for_meeting");
    assert!(attached, "segments attach to the meeting");
}

/// Complete a summary for the meeting through the real pipeline write path
/// (`update_process_completed` rewrites `result`; the 0033 generated column +
/// triggers index `$.markdown`).
async fn add_summary(pool: &SqlitePool, meeting_id: &str, markdown: &str) {
    SummaryProcessesRepository::create_or_reset_process(pool, meeting_id)
        .await
        .expect("create summary process");
    SummaryProcessesRepository::update_process_completed(
        pool,
        meeting_id,
        json!({ "markdown": markdown, "english_cache": null }),
        1,
        0.5,
        false,
        &app_lib::summary::refresh::generation_fingerprint(markdown),
        &app_lib::summary::refresh::speaker_names_fingerprint(std::iter::empty()),
    )
    .await
    .expect("complete summary");
}

// ---------------------------------------------------------------------------
// Fake LLM (records calls; scripted answers; optional mid-run cancellation)
// ---------------------------------------------------------------------------

type LlmFuture = Pin<Box<dyn Future<Output = Result<String, String>>>>;

/// Call-recording fake LLM. Map calls (any user prompt without the reduce
/// marker) answer from `map_answers` by matched substring; the reduce call
/// returns `reduce_answer`. Optionally cancels a token after the first call.
struct FakeLlm {
    map_answers: Vec<(&'static str, &'static str)>,
    reduce_answer: String,
    calls: Arc<Mutex<Vec<(String, String)>>>,
    cancel_after_first_call: Option<CancellationToken>,
}

impl FakeLlm {
    fn reduce_only(reduce_answer: &str) -> Self {
        Self {
            map_answers: Vec::new(),
            reduce_answer: reduce_answer.to_string(),
            calls: Arc::new(Mutex::new(Vec::new())),
            cancel_after_first_call: None,
        }
    }

    fn closure(&self) -> impl Fn(String, String) -> LlmFuture {
        let map_answers = self.map_answers.clone();
        let reduce_answer = self.reduce_answer.clone();
        let calls = self.calls.clone();
        let cancel = self.cancel_after_first_call.clone();
        move |system: String, user: String| {
            let map_answers = map_answers.clone();
            let reduce_answer = reduce_answer.clone();
            let calls = calls.clone();
            let cancel = cancel.clone();
            Box::pin(async move {
                calls.lock().unwrap().push((system.clone(), user.clone()));
                if let Some(token) = &cancel {
                    token.cancel();
                }
                if user.contains("Numbered meeting sources:") {
                    Ok(reduce_answer)
                } else {
                    let answer = map_answers
                        .iter()
                        .find(|(needle, _)| user.contains(needle))
                        .map(|(_, out)| out.to_string())
                        .unwrap_or_else(|| "NOTHING RELEVANT".to_string());
                    Ok(answer)
                }
            })
        }
    }

    fn calls(&self) -> Vec<(String, String)> {
        self.calls.lock().unwrap().clone()
    }
}

/// Progress collector for stage assertions.
fn collect_progress() -> (Arc<Mutex<Vec<Stage>>>, impl Fn(Stage)) {
    let stages = Arc::new(Mutex::new(Vec::new()));
    let sink = stages.clone();
    (stages, move |stage| sink.lock().unwrap().push(stage))
}

const GENEROUS_BUDGET: usize = 1_000_000; // forces single-pass

// ---------------------------------------------------------------------------
// (a) two summary meetings each hold half the answer -> both gathered + cited
// ---------------------------------------------------------------------------

#[tokio::test]
async fn answer_spanning_two_meetings_cites_both() {
    let (_dir, db) = fresh_db().await;
    let pool = db.pool();

    let m1 = create_meeting(pool, "Kickoff", "2026-06-20T10:00:00Z").await;
    let m2 = create_meeting(pool, "Pricing review", "2026-06-25T10:00:00Z").await;
    add_summary(
        pool,
        &m1,
        "## Decisions\nThe launch date was set to March 3.",
    )
    .await;
    add_summary(pool, &m2, "## Decisions\nPricing was set at $12 per seat.").await;
    // Noise meeting that matches nothing.
    let m3 = create_meeting(pool, "Standup", "2026-06-26T10:00:00Z").await;
    add_summary(pool, &m3, "## Notes\nBug triage only.").await;

    let question = "what did we decide about the launch date and pricing?";
    let gathered = gather(
        pool,
        question,
        &AggregationScope::default(),
        DEFAULT_MAX_MEETINGS,
    )
    .await
    .expect("gather");
    let ids: Vec<&str> = gathered
        .docs
        .iter()
        .map(|d| d.meeting_id.as_str())
        .collect();
    assert!(ids.contains(&m1.as_str()), "launch meeting gathered");
    assert!(ids.contains(&m2.as_str()), "pricing meeting gathered");
    assert!(!ids.contains(&m3.as_str()), "irrelevant meeting excluded");
    assert!(gathered.docs.iter().all(|d| d.source == DocSource::Summary));

    // Doc order is the [M#] numbering; build the fake answer to cite both.
    let m1_marker = format!("[M{}]", ids.iter().position(|id| *id == m1).unwrap() + 1);
    let m2_marker = format!("[M{}]", ids.iter().position(|id| *id == m2).unwrap() + 1);
    let reduce_answer =
        format!("Launch is March 3 {m1_marker}. Pricing is $12 per seat {m2_marker}.");

    let fake = FakeLlm::reduce_only(&reduce_answer);
    let cancel = CancellationToken::new();
    let (stages, on_progress) = collect_progress();

    let answer = execute_ask_ai(
        pool,
        question,
        &AggregationScope::default(),
        fake.closure(),
        GENEROUS_BUDGET,
        &cancel,
        on_progress,
    )
    .await
    .expect("execute_ask_ai");

    // One coherent markdown answer citing both meetings.
    assert!(answer.markdown.contains(&m1_marker));
    assert!(answer.markdown.contains(&m2_marker));
    let cited_of = |mid: &str| {
        answer
            .sources
            .iter()
            .find(|s| s.meeting_id == mid)
            .unwrap_or_else(|| panic!("{mid} in sources"))
            .cited
    };
    assert!(cited_of(&m1), "launch meeting cited");
    assert!(cited_of(&m2), "pricing meeting cited");

    // sources[i] <-> [M{i+1}]: the citation resolves to the right meeting.
    let idx1: usize = m1_marker[2..m1_marker.len() - 1].parse().unwrap();
    assert_eq!(answer.sources[idx1 - 1].meeting_id, m1);

    // Single pass with a generous budget: exactly one (reduce) LLM call.
    assert_eq!(fake.calls().len(), 1);
    let progress = stages.lock().unwrap().clone();
    assert_eq!(progress.first(), Some(&Stage::Gathering));
    assert_eq!(progress.last(), Some(&Stage::Reducing));
}

// ---------------------------------------------------------------------------
// (b) person + date scope filters exclude out-of-scope meetings
// ---------------------------------------------------------------------------

#[tokio::test]
async fn person_and_date_scopes_exclude_out_of_scope_meetings() {
    let (_dir, db) = fresh_db().await;
    let pool = db.pool();

    let m_old = create_meeting(pool, "Pricing kickoff", "2026-06-10T10:00:00Z").await;
    let m_new = create_meeting(pool, "Pricing follow-up", "2026-06-25T10:00:00Z").await;
    let m_spoke = create_meeting(pool, "Pricing retro", "2026-06-28T10:00:00Z").await;
    for id in [&m_old, &m_new, &m_spoke] {
        add_summary(pool, id, "We discussed the pricing page banner.").await;
    }

    // person-a is on m_old's roster; person-a also SPOKE in m_spoke (union
    // semantics: roster OR speaker).
    sqlx::query(
        "INSERT INTO meeting_participants (meeting_id, person_id, source, created_at) \
         VALUES (?, 'person-a', 'manual', '2026-06-10T10:00:00Z')",
    )
    .bind(&m_old)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO speakers (id, meeting_id, speaker_key, display_name, is_local, person_id, \
                               created_at, updated_at) \
         VALUES ('speaker-1', ?, 'spk_0', 'Ana', 0, 'person-a', \
                 '2026-06-28T10:00:00Z', '2026-06-28T10:00:00Z')",
    )
    .bind(&m_spoke)
    .execute(pool)
    .await
    .unwrap();

    let question = "what happened with the pricing page";

    // Unscoped: all three match.
    let all = gather(
        pool,
        question,
        &AggregationScope::default(),
        DEFAULT_MAX_MEETINGS,
    )
    .await
    .unwrap();
    assert_eq!(all.docs.len(), 3);

    // Date scope excludes the old meeting. Bounds are full ISO-8601 UTC
    // instants (the frontend sends local-midnight boundaries converted to UTC).
    let scope = AggregationScope {
        date_from: Some("2026-06-20T00:00:00Z".into()),
        ..Default::default()
    };
    let dated = gather(pool, question, &scope, DEFAULT_MAX_MEETINGS)
        .await
        .unwrap();
    let ids: Vec<&str> = dated.docs.iter().map(|d| d.meeting_id.as_str()).collect();
    assert_eq!(ids.len(), 2);
    assert!(
        !ids.contains(&m_old.as_str()),
        "m_old excluded by date_from"
    );

    // date_to is EXCLUSIVE — the start of the day after the selected end day.
    // End day 2026-06-25 (sent as 06-26T00:00Z) keeps m_new (06-25T10:00Z)…
    let scope = AggregationScope {
        date_to: Some("2026-06-26T00:00:00Z".into()),
        ..Default::default()
    };
    let until = gather(pool, question, &scope, DEFAULT_MAX_MEETINGS)
        .await
        .unwrap();
    let ids: Vec<&str> = until.docs.iter().map(|d| d.meeting_id.as_str()).collect();
    assert!(ids.contains(&m_new.as_str()), "end-day meeting included");
    assert!(!ids.contains(&m_spoke.as_str()), "06-28 excluded");
    // …while 06-25T00:00Z (end day 06-24) excludes it — instant comparison,
    // not day-granularity date().
    let scope = AggregationScope {
        date_to: Some("2026-06-25T00:00:00Z".into()),
        ..Default::default()
    };
    let until = gather(pool, question, &scope, DEFAULT_MAX_MEETINGS)
        .await
        .unwrap();
    let ids: Vec<&str> = until.docs.iter().map(|d| d.meeting_id.as_str()).collect();
    assert!(!ids.contains(&m_new.as_str()), "date_to is exclusive");
    assert!(ids.contains(&m_old.as_str()));

    // Person scope: roster (m_old) UNION spoke (m_spoke); m_new excluded.
    let scope = AggregationScope {
        person_id: Some("person-a".into()),
        ..Default::default()
    };
    let person = gather(pool, question, &scope, DEFAULT_MAX_MEETINGS)
        .await
        .unwrap();
    let ids: Vec<&str> = person.docs.iter().map(|d| d.meeting_id.as_str()).collect();
    assert_eq!(ids.len(), 2, "roster + speaker meetings only");
    assert!(ids.contains(&m_old.as_str()));
    assert!(ids.contains(&m_spoke.as_str()));
    assert!(!ids.contains(&m_new.as_str()));

    // Combined person + date narrows to the intersection.
    let scope = AggregationScope {
        person_id: Some("person-a".into()),
        date_from: Some("2026-06-20T00:00:00Z".into()),
        ..Default::default()
    };
    let both = gather(pool, question, &scope, DEFAULT_MAX_MEETINGS)
        .await
        .unwrap();
    assert_eq!(both.docs.len(), 1);
    assert_eq!(both.docs[0].meeting_id, m_spoke);

    // The scope propagates to the answer's source list.
    let fake = FakeLlm::reduce_only("Discussed the banner [M1].");
    let cancel = CancellationToken::new();
    let answer = execute_ask_ai(
        pool,
        question,
        &scope,
        fake.closure(),
        GENEROUS_BUDGET,
        &cancel,
        |_| {},
    )
    .await
    .unwrap();
    assert_eq!(answer.sources.len(), 1);
    assert_eq!(answer.sources[0].meeting_id, m_spoke);
}

// ---------------------------------------------------------------------------
// (c) summaries-first policy + bounded transcript excerpt
// ---------------------------------------------------------------------------

#[tokio::test]
async fn summary_body_excludes_transcript_and_excerpt_covers_transcript_only_evidence() {
    let (_dir, db) = fresh_db().await;
    let pool = db.pool();

    // Case 1: evidence lives in the summary and the transcript never matches
    // the question at all. The transcript must contribute nothing — no body,
    // no excerpt (there is no transcript hit to window around).
    let m1 = create_meeting(pool, "Kickoff", "2026-06-20T10:00:00Z").await;
    add_transcript(
        pool,
        &m1,
        "Kickoff",
        &["completely unrelated chatter about the office espresso machine"],
    )
    .await;
    add_summary(pool, &m1, "Decision: adopt the flamingo branding.").await;

    let gathered = gather(
        pool,
        "what about the flamingo branding",
        &AggregationScope::default(),
        DEFAULT_MAX_MEETINGS,
    )
    .await
    .unwrap();
    assert_eq!(gathered.docs.len(), 1);
    let doc = &gathered.docs[0];
    assert_eq!(doc.source, DocSource::Summary);
    assert!(doc.text.contains("flamingo branding"));
    assert!(
        !doc.full_text().contains("espresso machine"),
        "a transcript with no hit for the question must NOT be included when a summary exists"
    );
    assert!(
        doc.excerpt.is_none(),
        "no excerpt when the question has no transcript hit"
    );

    // Case 2: evidence phrase was SAID but never made the summary — the doc
    // body stays the summary, and a bounded transcript excerpt is appended.
    let m2 = create_meeting(pool, "Retro", "2026-06-25T10:00:00Z").await;
    add_transcript(
        pool,
        &m2,
        "Retro",
        &[
            "intro chatter before the main topic",
            "we agreed the zanzibar rollout starts next quarter",
            "closing remarks and goodbyes",
        ],
    )
    .await;
    add_summary(pool, &m2, "General retro notes about test flakiness.").await;

    let gathered = gather(
        pool,
        "when does the zanzibar rollout start",
        &AggregationScope::default(),
        DEFAULT_MAX_MEETINGS,
    )
    .await
    .unwrap();
    assert_eq!(gathered.docs.len(), 1);
    let doc = &gathered.docs[0];
    assert_eq!(doc.meeting_id, m2);
    assert_eq!(doc.source, DocSource::Summary, "body is still the summary");
    assert!(doc.text.contains("test flakiness"));
    assert!(
        !doc.text.contains("zanzibar"),
        "the summary body itself does not contain the transcript-only phrase"
    );
    let excerpt = doc
        .excerpt
        .as_deref()
        .expect("transcript-only evidence appends a bounded excerpt");
    assert!(
        excerpt.contains("zanzibar rollout"),
        "excerpt must contain the evidence, got: {excerpt}"
    );
    // full_text() folds the excerpt in for the model.
    assert!(doc.full_text().contains("zanzibar rollout"));
}

// ---------------------------------------------------------------------------
// (c3) specs/0056 W3: transcript evidence is attached even when the SUMMARY is
// the meeting's best hit. bm25 is not comparable across the three FTS tables,
// so "which source ranked best" says nothing about where the evidence lives —
// a short, dense summary out-ranks one transcript segment on generic terms.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn transcript_evidence_is_attached_with_speaker_when_the_summary_ranks_best() {
    let (_dir, db) = fresh_db().await;
    let pool = db.pool();

    // Noise: other summarized meetings that share none of the question's
    // terms. They give the summaries index a real idf (FTS5 clamps a lone
    // all-matching document's idf to ~0), so the target summary can out-rank
    // the transcript the way it does in a populated database.
    for (i, text) in [
        "Budget review: the quarterly numbers were approved without changes.",
        "Design critique of the onboarding flow; two follow-ups for next week.",
        "Hiring pipeline update and interview loop tweaks.",
    ]
    .iter()
    .enumerate()
    {
        let m = create_meeting(pool, &format!("Noise {i}"), "2026-08-20T10:00:00Z").await;
        add_summary(pool, &m, text).await;
    }

    // The owner's repro (specs/0056 item 5): "in what meeting did Arno mention
    // that he talked to Frank?" The summary is dense on the generic question
    // terms (Arno, mention) and never says a word about Frank; the transcript
    // carries the actual evidence, spoken by a diarized speaker named Arno.
    let m = create_meeting(pool, "Weekly sync", "2026-08-28T10:00:00Z").await;
    let mut segments: Vec<TranscriptSegment> = vec![
        segment("okay let's get started with the weekly sync", 0.0, 2.0),
        segment(
            "I talked to Frank yesterday and he is fine with the budget",
            2.0,
            4.0,
        ),
        segment("great, next item is the roadmap", 4.0, 6.0),
    ];
    segments[1].speaker = Some("spk_0".into());
    let attached = TranscriptsRepository::save_transcripts_for_meeting(
        pool,
        &m,
        "Weekly sync",
        &segments,
        None,
    )
    .await
    .expect("save_transcripts_for_meeting");
    assert!(attached);
    SpeakersRepository::upsert(pool, &m, "spk_0", "Arno", false, None, None, None)
        .await
        .expect("speaker row");
    add_summary(
        pool,
        &m,
        "Arno led the sync. Arno asked the team to mention blockers early; two people \
         did mention roadmap risks, and Arno will follow up on both.",
    )
    .await;

    let question = "in what meeting did Arno mention that he talked to Frank?";

    // Fixture sanity: the summary IS the meeting's best hit (exactly the case
    // the old policy dropped), while the transcript still has hits of its own.
    let ranking = SearchRepository::rank_meetings_for_question(
        pool,
        question,
        &AggregationScope::default(),
        DEFAULT_MAX_MEETINGS,
    )
    .await
    .unwrap();
    assert_eq!(ranking.meetings.len(), 1, "only the target meeting matches");
    assert_eq!(
        ranking.meetings[0].best_source.as_deref(),
        Some("summary"),
        "fixture: the summary must out-rank the transcript for this test to bite"
    );
    assert!(
        !ranking.meetings[0].transcript_hit_ids.is_empty(),
        "the transcript's own hits are recorded regardless of the winner"
    );

    let gathered = gather(
        pool,
        question,
        &AggregationScope::default(),
        DEFAULT_MAX_MEETINGS,
    )
    .await
    .unwrap();
    assert_eq!(gathered.docs.len(), 1);
    let doc = &gathered.docs[0];
    assert_eq!(doc.meeting_id, m);
    assert_eq!(doc.source, DocSource::Summary, "body is still the summary");
    assert!(
        !doc.text.contains("Frank"),
        "the summary body itself never mentions Frank"
    );
    let excerpt = doc
        .excerpt
        .as_deref()
        .expect("transcript hits attach an excerpt even though the summary ranked best");
    let evidence = "Arno: I talked to Frank yesterday and he is fine with the budget";
    assert!(
        excerpt.contains(evidence),
        "excerpt lines carry the resolved speaker name, got:\n{excerpt}"
    );
    assert!(
        !excerpt.contains("[…]"),
        "one contiguous window has no gap marker, got:\n{excerpt}"
    );
    assert!(
        doc.full_text().contains(evidence),
        "full_text() is what the engine packs into the map prompt"
    );

    // And the map prompt the model actually receives carries the line.
    let fake = FakeLlm::reduce_only("Arno mentioned it in the weekly sync [M1].");
    let cancel = CancellationToken::new();
    execute_ask_ai(
        pool,
        question,
        &AggregationScope::default(),
        fake.closure(),
        GENEROUS_BUDGET,
        &cancel,
        |_| {},
    )
    .await
    .expect("execute_ask_ai");
    assert!(
        fake.calls().iter().any(|(_, user)| user.contains(evidence)),
        "a map prompt must contain the speaker-labelled evidence line"
    );
}

// ---------------------------------------------------------------------------
// (c2) consent pinning: explicit meeting_ids keep FTS fidelity (R2)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn pinned_meeting_ids_keep_transcript_excerpts_and_never_widen_the_set() {
    let (_dir, db) = fresh_db().await;
    let pool = db.pool();

    // Evidence phrase said in the meeting but absent from the summary — the
    // excerpt policy's case — plus an unrelated meeting OUTSIDE the pinned set
    // that also matches the question.
    let m_pinned = create_meeting(pool, "Retro", "2026-06-25T10:00:00Z").await;
    add_transcript(
        pool,
        &m_pinned,
        "Retro",
        &[
            "intro chatter before the main topic",
            "we agreed the zanzibar rollout starts next quarter",
            "closing remarks and goodbyes",
        ],
    )
    .await;
    add_summary(pool, &m_pinned, "General retro notes about test flakiness.").await;

    let m_outside = create_meeting(pool, "Other sync", "2026-06-26T10:00:00Z").await;
    add_summary(
        pool,
        &m_outside,
        "The zanzibar rollout was also mentioned here.",
    )
    .await;

    let question = "when does the zanzibar rollout start";
    let scope = AggregationScope {
        meeting_ids: Some(vec![m_pinned.clone()]),
        ..Default::default()
    };

    let gathered = gather(pool, question, &scope, DEFAULT_MAX_MEETINGS)
        .await
        .unwrap();

    // The set is pinned: the matching outside meeting is never pulled in.
    assert_eq!(gathered.docs.len(), 1, "pinned scope fixes the set");
    assert_eq!(gathered.dropped, 0, "pinned scopes never report dropped");
    let doc = &gathered.docs[0];
    assert_eq!(doc.meeting_id, m_pinned);
    assert_eq!(doc.source, DocSource::Summary, "body is still the summary");
    // FTS fidelity within the pinned set: the transcript-only evidence still
    // arrives as a bounded excerpt, identical to the unpinned preview's doc.
    let excerpt = doc
        .excerpt
        .as_deref()
        .expect("pinned ids must keep the transcript-excerpt policy");
    assert!(
        excerpt.contains("zanzibar rollout"),
        "excerpt must contain the evidence, got: {excerpt}"
    );
}

// ---------------------------------------------------------------------------
// (d) no-summary meeting: notes fallback, then transcript fallback
// ---------------------------------------------------------------------------

#[tokio::test]
async fn no_summary_meeting_falls_back_to_notes_then_transcript() {
    let (_dir, db) = fresh_db().await;
    let pool = db.pool();

    // Notes fallback: no summary, has notes AND a transcript -> notes win.
    let m_notes = create_meeting(pool, "Planning", "2026-06-20T10:00:00Z").await;
    add_transcript(
        pool,
        &m_notes,
        "Planning",
        &["spoken discussion about the quokka milestone dates"],
    )
    .await;
    assert!(MeetingNotesRepository::upsert_notes(
        pool,
        &m_notes,
        Some("# Plan\nquokka milestone locked for August"),
        None,
    )
    .await
    .unwrap());

    let gathered = gather(
        pool,
        "what is the quokka milestone",
        &AggregationScope::default(),
        DEFAULT_MAX_MEETINGS,
    )
    .await
    .unwrap();
    assert_eq!(gathered.docs.len(), 1);
    let doc = &gathered.docs[0];
    assert_eq!(
        doc.source,
        DocSource::Notes,
        "notes beat the raw transcript"
    );
    assert!(doc.text.contains("quokka milestone locked for August"));

    // Transcript fallback: no summary, no notes -> raw transcript is the body.
    let m_raw = create_meeting(pool, "Hallway chat", "2026-06-25T10:00:00Z").await;
    add_transcript(
        pool,
        &m_raw,
        "Hallway chat",
        &["only the transcript mentions the wombat migration plan"],
    )
    .await;

    let gathered = gather(
        pool,
        "what about the wombat migration",
        &AggregationScope::default(),
        DEFAULT_MAX_MEETINGS,
    )
    .await
    .unwrap();
    assert_eq!(gathered.docs.len(), 1);
    let doc = &gathered.docs[0];
    assert_eq!(doc.meeting_id, m_raw);
    assert_eq!(doc.source, DocSource::Transcript);
    assert!(doc.text.contains("wombat migration plan"));
    assert!(
        doc.excerpt.is_none(),
        "no excerpt when the body already IS the transcript"
    );
}

// ---------------------------------------------------------------------------
// (e) token-estimate parity + over-cap dropped count
// ---------------------------------------------------------------------------

#[tokio::test]
async fn estimate_covers_what_a_single_pass_run_actually_sends() {
    let (_dir, db) = fresh_db().await;
    let pool = db.pool();

    let m1 = create_meeting(pool, "Kickoff", "2026-06-20T10:00:00Z").await;
    let m2 = create_meeting(pool, "Retro", "2026-06-25T10:00:00Z").await;
    add_summary(pool, &m1, "The gondola project ships in March.").await;
    add_summary(pool, &m2, "Gondola pricing set at $9 per seat.").await;

    let question = "what did we decide about the gondola project?";
    let scope = AggregationScope::default();

    let gathered = gather(pool, question, &scope, DEFAULT_MAX_MEETINGS)
        .await
        .unwrap();
    assert_eq!(gathered.docs.len(), 2);
    let estimate = estimate_run_tokens(&ask_ai_prompt(question), &gathered.docs);
    assert!(estimate > 0);

    // Run single-pass and check the estimate covers the prompt actually sent
    // (estimate >= the real prompt's tokens; the delta is the fixed overhead
    // reserve `run` budgets with).
    let fake = FakeLlm::reduce_only("Ships in March [M1] at $9 [M2].");
    let cancel = CancellationToken::new();
    let answer = execute_ask_ai(
        pool,
        question,
        &scope,
        fake.closure(),
        GENEROUS_BUDGET,
        &cancel,
        |_| {},
    )
    .await
    .unwrap();
    assert_eq!(answer.sources.len(), 2);

    let calls = fake.calls();
    assert_eq!(calls.len(), 1, "single pass");
    let (system, user) = &calls[0];
    let sent_tokens = rough_token_count(system) + rough_token_count(user);
    assert!(
        estimate >= sent_tokens,
        "estimate ({estimate}) covers what was actually sent ({sent_tokens})"
    );
    assert!(
        estimate - sent_tokens <= 400,
        "estimate is the sent prompt plus only the fixed overhead reserve"
    );
}

#[tokio::test]
async fn gather_over_the_cap_reports_dropped() {
    let (_dir, db) = fresh_db().await;
    let pool = db.pool();

    // 12 matching meetings against the 10-meeting cap -> top 10, dropped 2.
    for i in 0..12 {
        let id = create_meeting(
            pool,
            &format!("Sync {i}"),
            &format!("2026-06-{:02}T10:00:00Z", i + 1),
        )
        .await;
        add_summary(pool, &id, "Recurring capybara project status update.").await;
    }
    let gathered = gather(
        pool,
        "status of the capybara project",
        &AggregationScope::default(),
        DEFAULT_MAX_MEETINGS,
    )
    .await
    .unwrap();
    assert_eq!(gathered.docs.len(), DEFAULT_MAX_MEETINGS);
    assert_eq!(
        gathered.dropped, 2,
        "matched beyond the cap surfaces as dropped"
    );
}

// ---------------------------------------------------------------------------
// (f) cancellation mid-map: prompt stop, no reduce call
// ---------------------------------------------------------------------------

#[tokio::test]
async fn cancellation_mid_map_stops_promptly_without_a_reduce_call() {
    let (_dir, db) = fresh_db().await;
    let pool = db.pool();

    // Two meetings with bodies large enough that a small budget forces
    // map-reduce (each doc still fits its own map call).
    let filler = "budget planning discussion item ".repeat(80);
    let m1 = create_meeting(pool, "Kickoff", "2026-06-20T10:00:00Z").await;
    let m2 = create_meeting(pool, "Retro", "2026-06-25T10:00:00Z").await;
    add_summary(pool, &m1, &format!("Budget decisions part one. {filler}")).await;
    add_summary(pool, &m2, &format!("Budget decisions part two. {filler}")).await;

    let question = "what were the budget decisions?";
    let gathered = gather(
        pool,
        question,
        &AggregationScope::default(),
        DEFAULT_MAX_MEETINGS,
    )
    .await
    .unwrap();
    assert_eq!(gathered.docs.len(), 2);
    // Budget below the single-pass need (probe = both docs + overhead) but big
    // enough for one doc's map call: force the map stage without chunking.
    let prompt = ask_ai_prompt(question);
    let budget = estimate_run_tokens(&prompt, &gathered.docs[..1]) + 200;
    assert!(
        budget < estimate_run_tokens(&prompt, &gathered.docs),
        "budget must force map-reduce"
    );

    let cancel = CancellationToken::new();
    let fake = FakeLlm {
        map_answers: vec![("Budget decisions", "- budget extract")],
        reduce_answer: "NEVER REACHED".to_string(),
        calls: Arc::new(Mutex::new(Vec::new())),
        cancel_after_first_call: Some(cancel.clone()),
    };
    let (stages, on_progress) = collect_progress();

    let err = execute_ask_ai(
        pool,
        question,
        &AggregationScope::default(),
        fake.closure(),
        budget,
        &cancel,
        on_progress,
    )
    .await
    .expect_err("cancelled run must error");

    assert!(
        err.to_string().contains("cancelled"),
        "cancellation surfaces as a 'cancelled' error, got: {err}"
    );
    let calls = fake.calls();
    assert_eq!(calls.len(), 1, "stops after the in-flight map call");
    assert!(
        !calls[0].1.contains("Numbered meeting sources:"),
        "the one call was a map, never the reduce"
    );
    let progress = stages.lock().unwrap().clone();
    assert!(
        !progress.contains(&Stage::Reducing),
        "no Reducing progress after a mid-map cancellation"
    );
}

// ---------------------------------------------------------------------------
// Consumer-agnostic contract: the engine with a NON-Ask prompt
// ---------------------------------------------------------------------------

#[tokio::test]
async fn engine_serves_a_non_ask_prompt_over_gathered_docs() {
    let (_dir, db) = fresh_db().await;
    let pool = db.pool();

    let m1 = create_meeting(pool, "Weekly sync", "2026-06-20T10:00:00Z").await;
    let m2 = create_meeting(pool, "Weekly sync", "2026-06-27T10:00:00Z").await;
    add_summary(pool, &m1, "Migration progress: schema drafted.").await;
    add_summary(pool, &m2, "Migration progress: schema applied.").await;

    // A topic roll-up style consumer (0013 3b shape): its own prompt pair over
    // the SAME gather + run, no Ask-AI code involved.
    let rollup = AggregationPrompt {
        map_system: "You extract status updates about a topic from one meeting.".to_string(),
        map_user: Box::new(|doc: &MeetingDoc| {
            format!(
                "TOPIC: migration\nMEETING: {}\n{}",
                doc.title,
                doc.full_text()
            )
        }),
        reduce_system: "You write a topic roll-up from the numbered extracts.".to_string(),
        reduce_user: Box::new(|block: &str| format!("ROLLUP OVER:\n{block}")),
    };

    let gathered = gather(
        pool,
        "migration schema progress",
        &AggregationScope::default(),
        DEFAULT_MAX_MEETINGS,
    )
    .await
    .unwrap();
    assert_eq!(gathered.docs.len(), 2);

    let fake = FakeLlm::reduce_only("Schema drafted [M1], then applied [M2].");
    // The generic reduce marker differs from Ask-AI's — route on this prompt's
    // own shape instead.
    let calls = fake.calls.clone();
    let llm = move |system: String, user: String| -> LlmFuture {
        let calls = calls.clone();
        Box::pin(async move {
            calls.lock().unwrap().push((system, user.clone()));
            if user.starts_with("ROLLUP OVER:") {
                Ok("Schema drafted [M1], then applied [M2].".to_string())
            } else {
                Ok("- migration status line".to_string())
            }
        })
    };

    let cancel = CancellationToken::new();
    let answer = run(
        llm,
        &rollup,
        &gathered.docs,
        GENEROUS_BUDGET,
        &cancel,
        |_| {},
    )
    .await
    .expect("non-Ask consumer runs on the same engine");

    assert_eq!(answer.sources.len(), 2);
    assert!(answer.sources[0].cited && answer.sources[1].cited);
    let calls = fake.calls();
    assert_eq!(calls.len(), 1, "single pass for the roll-up too");
    assert_eq!(
        calls[0].0,
        "You write a topic roll-up from the numbered extracts."
    );
    assert!(calls[0].1.starts_with("ROLLUP OVER:"));
    assert!(calls[0].1.contains("[M1] \"Weekly sync\""));
    assert!(calls[0].1.contains("[M2] \"Weekly sync\""));
}
