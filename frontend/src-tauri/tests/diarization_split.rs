//! specs/0044 W1.2 — DB round-trip for splitting transcript rows that straddle a
//! speaker change. The pure planning logic is unit-tested in `diarization::split`;
//! these tests prove the persistence contract: the first part keeps the original
//! row id (overrides/FTS stay coherent), later parts are new rows, protected rows
//! are never touched, and re-runs are idempotent. No models, no audio.
//!
//! Run with:
//!   cargo test --features metal --test diarization_split -- --nocapture

mod common;

use app_lib::database::repositories::meeting::MeetingsRepository;
use app_lib::database::repositories::transcript::TranscriptsRepository;
use app_lib::database::repositories::transcript_speaker_overrides::TranscriptSpeakerOverridesRepository;
use app_lib::diarization::align::pad_trimmed;
use app_lib::diarization::segments::load_segments;
use app_lib::diarization::split::split_straddling_rows;
use app_lib::diarization::{
    align_turns_to_segments, AlignableSegment, SpeakerTurn, LOCAL_SPEAKER_KEY,
};
use common::{fresh_db, segment};

fn turn(start: f32, end: f32, speaker: &str) -> SpeakerTurn {
    SpeakerTurn {
        start,
        end,
        speaker: speaker.to_string(),
    }
}

/// Two-speaker fast handoff bridged by VAD redemption into one stored row.
const MERGED_TEXT: &str = "so that works for me. Great lets ship it tomorrow then";

fn handoff_turns() -> Vec<SpeakerTurn> {
    vec![turn(0.0, 3.0, "spk_0"), turn(3.4, 6.3, "spk_1")]
}

async fn seed_meeting_with_row(
    pool: &sqlx::SqlitePool,
    text: &str,
    start: f64,
    end: f64,
) -> (String, String) {
    let meeting_id = MeetingsRepository::create_meeting(pool, None, None, None, None, None)
        .await
        .expect("create_meeting");
    let segs = vec![segment(text, start, end)];
    TranscriptsRepository::save_transcripts_for_meeting(pool, &meeting_id, "Split", &segs, None)
        .await
        .expect("save segments");
    let row_id: String = sqlx::query_scalar("SELECT id FROM transcripts WHERE meeting_id = ?")
        .bind(&meeting_id)
        .fetch_one(pool)
        .await
        .expect("row id");
    (meeting_id, row_id)
}

#[tokio::test]
async fn straddling_row_is_split_and_parts_persisted() {
    let (_dir, db) = fresh_db().await;
    let pool = db.pool();
    let (meeting_id, original_id) = seed_meeting_with_row(pool, MERGED_TEXT, 0.0, 6.7).await;

    let n = split_straddling_rows(pool, &meeting_id, &handoff_turns())
        .await
        .expect("split");
    assert_eq!(n, 1, "exactly one row should split");

    let rows: Vec<(String, String, f64, f64, Option<f64>)> = sqlx::query_as(
        "SELECT id, transcript, audio_start_time, audio_end_time, duration
         FROM transcripts WHERE meeting_id = ? ORDER BY audio_start_time",
    )
    .bind(&meeting_id)
    .fetch_all(pool)
    .await
    .expect("rows");
    assert_eq!(rows.len(), 2, "one row became two parts");

    // First part keeps the ORIGINAL id (specs/0019: override ids stay stable) and
    // shrinks to the boundary; the tail is a new row covering the rest.
    let (first_id, first_text, first_start, first_end, first_dur) = &rows[0];
    assert_eq!(first_id, &original_id);
    assert_eq!(first_text, "so that works for me. Great");
    assert!((first_start - 0.0).abs() < 1e-4);
    assert!((first_end - 3.2).abs() < 1e-3, "boundary was {first_end}");
    assert!((first_dur.unwrap() - 3.2).abs() < 1e-3);

    let (second_id, second_text, second_start, second_end, _) = &rows[1];
    assert_ne!(second_id, &original_id);
    assert!(second_id.starts_with("transcript-"));
    assert_eq!(second_text, "lets ship it tomorrow then");
    assert!(
        (second_start - first_end).abs() < 1e-6,
        "parts are contiguous"
    );
    assert!((second_end - 6.7).abs() < 1e-4);

    // FTS stayed coherent through the UPDATE + INSERT (triggers fired): both
    // parts are findable, and the pre-split phrasing spanning the cut is gone.
    let hits: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM transcripts_fts WHERE transcripts_fts MATCH 'tomorrow'",
    )
    .fetch_one(pool)
    .await
    .expect("fts query");
    assert_eq!(hits, 1);
    let hits: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM transcripts_fts WHERE transcripts_fts MATCH 'works'",
    )
    .fetch_one(pool)
    .await
    .expect("fts query");
    assert_eq!(hits, 1);
}

/// specs/0046 WS2 critical fix regression (end-to-end): a straddling row that
/// starts well into the recording (not at t=0 — the case every earlier test
/// missed) with real `word_timestamps` set must still split at the exact word
/// edge through the full DB path, proving `common.rs`'s offset-to-recording-
/// relative fix and `split.rs`'s word-exact consumption agree on one origin.
#[tokio::test]
async fn straddling_row_with_word_timestamps_splits_at_word_edge() {
    let (_dir, db) = fresh_db().await;
    let pool = db.pool();
    const OFFSET: f64 = 120.0;
    let (meeting_id, row_id) = seed_meeting_with_row(pool, MERGED_TEXT, OFFSET, OFFSET + 6.7).await;

    // Recording-relative word timestamps (same words/timings as split.rs's
    // word-edge unit tests), shifted by OFFSET onto the row's audio_start_time
    // origin — exactly what create_transcript_segments_with_words now produces.
    let raw_words: &[(&str, f64, f64)] = &[
        ("so", 0.0, 0.2),
        ("that", 0.2, 0.5),
        ("works", 0.5, 0.9),
        ("for", 0.9, 1.1),
        ("me.", 1.1, 3.0),
        ("Great", 3.4, 3.8),
        ("lets", 3.8, 4.0),
        ("ship", 4.0, 4.3),
        ("it", 4.3, 4.5),
        ("tomorrow", 4.5, 5.0),
        ("then", 5.0, 6.7),
    ];
    let words_json = serde_json::to_string(
        &raw_words
            .iter()
            .map(|&(w, s, e)| serde_json::json!({"w": w, "s": OFFSET + s, "e": OFFSET + e}))
            .collect::<Vec<_>>(),
    )
    .expect("serialize word_timestamps");
    sqlx::query("UPDATE transcripts SET word_timestamps = ? WHERE id = ?")
        .bind(&words_json)
        .bind(&row_id)
        .execute(pool)
        .await
        .expect("seed word_timestamps");

    let turns = vec![
        turn(OFFSET as f32, OFFSET as f32 + 3.0, "spk_0"),
        turn(OFFSET as f32 + 3.4, OFFSET as f32 + 6.3, "spk_1"),
    ];
    let n = split_straddling_rows(pool, &meeting_id, &turns)
        .await
        .expect("split");
    assert_eq!(n, 1, "the word-timestamped straddling row should split");

    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT id, transcript FROM transcripts WHERE meeting_id = ? ORDER BY audio_start_time",
    )
    .bind(&meeting_id)
    .fetch_all(pool)
    .await
    .expect("rows");
    assert_eq!(rows.len(), 2, "one row became two parts");
    assert_eq!(
        rows[0].1, "so that works for me.",
        "word-exact split, not the char-proportional fallback's 'Great' spillover"
    );
    assert_eq!(rows[1].1, "Great lets ship it tomorrow then");
}

#[tokio::test]
async fn overridden_row_is_never_split() {
    let (_dir, db) = fresh_db().await;
    let pool = db.pool();
    let (meeting_id, row_id) = seed_meeting_with_row(pool, MERGED_TEXT, 0.0, 6.7).await;

    // The owner hand-corrected this line (specs/0019 WS2.3): its id ↔ span
    // meaning must stay stable, so the splitter must leave it whole.
    let applied = TranscriptSpeakerOverridesRepository::set(pool, &meeting_id, &row_id, "spk_0")
        .await
        .expect("set override");
    assert!(applied);

    let n = split_straddling_rows(pool, &meeting_id, &handoff_turns())
        .await
        .expect("split");
    assert_eq!(n, 0, "overridden row must not split");

    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM transcripts WHERE meeting_id = ?")
        .bind(&meeting_id)
        .fetch_one(pool)
        .await
        .expect("count");
    assert_eq!(count, 1);
}

#[tokio::test]
async fn mic_row_is_never_split() {
    let (_dir, db) = fresh_db().await;
    let pool = db.pool();
    let (meeting_id, row_id) = seed_meeting_with_row(pool, MERGED_TEXT, 0.0, 6.7).await;
    sqlx::query("UPDATE transcripts SET channel = 'microphone' WHERE id = ?")
        .bind(&row_id)
        .execute(pool)
        .await
        .expect("tag mic");

    let n = split_straddling_rows(pool, &meeting_id, &handoff_turns())
        .await
        .expect("split");
    assert_eq!(n, 0, "mic rows are single-speaker by construction");
}

#[tokio::test]
async fn rerun_is_idempotent() {
    let (_dir, db) = fresh_db().await;
    let pool = db.pool();
    let (meeting_id, _) = seed_meeting_with_row(pool, MERGED_TEXT, 0.0, 6.7).await;

    let first = split_straddling_rows(pool, &meeting_id, &handoff_turns())
        .await
        .expect("split");
    assert_eq!(first, 1);
    let second = split_straddling_rows(pool, &meeting_id, &handoff_turns())
        .await
        .expect("re-split");
    assert_eq!(second, 0, "already-tight parts must not split again");

    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM transcripts WHERE meeting_id = ?")
        .bind(&meeting_id)
        .fetch_one(pool)
        .await
        .expect("count");
    assert_eq!(count, 2);
}

// ---------------------------------------------------------------------------
// specs/0046 W1.4 — both-direction owner<->remote split integration.
//
// Tasks 1-3 taught `split_straddling_rows` to tag each part's `channel` by the
// speaker run that owns it (owner run -> "microphone", spk_N run -> "system"),
// which is what lets the normal alignment pass (`load_segments` +
// `align_turns_to_segments`) label both halves of a glued owner<->remote row
// correctly. These tests prove that end-to-end, through the real DB, in BOTH
// speaker orders, plus the override safety rail on the same kind of fixture.
// ---------------------------------------------------------------------------

/// Tag `row_id`'s capture channel (mirrors the RMS-dominance tag recorded at
/// capture time, specs/0029 WS3.4).
async fn tag_channel(pool: &sqlx::SqlitePool, row_id: &str, channel: &str) {
    sqlx::query("UPDATE transcripts SET channel = ? WHERE id = ?")
        .bind(channel)
        .bind(row_id)
        .execute(pool)
        .await
        .expect("tag channel");
}

/// Re-derive each of the meeting's transcript rows' aligned speaker key, in the
/// same order `load_segments` returns them (audio_start_time ascending). Mirrors
/// `pipeline::run`'s alignment step exactly: pad-trim each row's stored span
/// before scoring overlap, then `align_turns_to_segments`.
async fn aligned_keys(
    pool: &sqlx::SqlitePool,
    meeting_id: &str,
    turns: &[SpeakerTurn],
) -> Vec<String> {
    let segments = load_segments(pool, meeting_id)
        .await
        .expect("load_segments");
    let alignable: Vec<AlignableSegment> = segments
        .iter()
        .map(|s| {
            let (ts, te) = pad_trimmed(s.start, s.end);
            AlignableSegment::new(ts, te, s.channel)
        })
        .collect();
    align_turns_to_segments(turns, &alignable)
}

#[tokio::test]
async fn system_tagged_row_is_not_carved_into_you_by_an_owner_turn() {
    // specs/0047 W2: a system-tagged (RMS: remote-dominant) row must never be
    // carved so that half becomes "You". On speakers, a "local" turn straddling it
    // is mic-bleed of the remote voice, not the owner — trusting it would carve a
    // spurious "You" clip out of the middle of someone else's turn (the reported
    // symptom). The row stays whole and all-remote. (Owner turns still split the
    // owner's OWN mic-tagged rows — see the microphone-tagged test below.)
    let (_dir, db) = fresh_db().await;
    let pool = db.pool();
    let (meeting_id, row_id) = seed_meeting_with_row(pool, MERGED_TEXT, 0.0, 6.7).await;
    tag_channel(pool, &row_id, "system").await;

    let turns = vec![turn(0.0, 3.0, "spk_0"), turn(3.4, 6.3, LOCAL_SPEAKER_KEY)];

    let n = split_straddling_rows(pool, &meeting_id, &turns)
        .await
        .expect("split");
    assert_eq!(n, 0, "a system-tagged row must not be carved by an owner turn");

    let rows: Vec<(String, Option<String>)> = sqlx::query_as(
        "SELECT id, channel FROM transcripts WHERE meeting_id = ? ORDER BY audio_start_time",
    )
    .bind(&meeting_id)
    .fetch_all(pool)
    .await
    .expect("rows");
    assert_eq!(rows.len(), 1, "the row stays whole");
    assert_eq!(rows[0].1.as_deref(), Some("system"));

    let keys = aligned_keys(pool, &meeting_id, &turns).await;
    assert_eq!(
        keys,
        vec!["spk_0".to_string()],
        "the whole remote row stays with the remote speaker, never {LOCAL_SPEAKER_KEY}"
    );
}

#[tokio::test]
async fn system_tagged_remote_to_remote_row_still_splits() {
    // Guard: excluding owner turns for non-mic rows must NOT disable
    // remote<->remote splitting. A system-tagged row straddling two remote
    // speakers still splits into two system parts (specs/0044 preserved).
    let (_dir, db) = fresh_db().await;
    let pool = db.pool();
    let (meeting_id, row_id) = seed_meeting_with_row(pool, MERGED_TEXT, 0.0, 6.7).await;
    tag_channel(pool, &row_id, "system").await;

    let n = split_straddling_rows(pool, &meeting_id, &handoff_turns())
        .await
        .expect("split");
    assert_eq!(n, 1, "remote<->remote handoff still splits");

    let channels: Vec<Option<String>> = sqlx::query_scalar::<_, Option<String>>(
        "SELECT channel FROM transcripts WHERE meeting_id = ? ORDER BY audio_start_time",
    )
    .bind(&meeting_id)
    .fetch_all(pool)
    .await
    .expect("rows");
    assert_eq!(channels.len(), 2);
    assert!(
        channels.iter().all(|c| c.as_deref() == Some("system")),
        "both remote parts stay system"
    );
}

#[tokio::test]
async fn microphone_tagged_glued_row_splits_and_aligns_owner_then_remote() {
    let (_dir, db) = fresh_db().await;
    let pool = db.pool();
    let (meeting_id, row_id) = seed_meeting_with_row(pool, MERGED_TEXT, 0.0, 6.7).await;
    tag_channel(pool, &row_id, "microphone").await;

    // [owner tail (local) + Person A onset (spk_0)] glued into one mic-tagged row.
    let turns = vec![turn(0.0, 3.0, LOCAL_SPEAKER_KEY), turn(3.4, 6.3, "spk_0")];

    let n = split_straddling_rows(pool, &meeting_id, &turns)
        .await
        .expect("split");
    assert_eq!(n, 1, "the straddling mic-tagged row should split");

    let rows: Vec<(String, Option<String>)> = sqlx::query_as(
        "SELECT id, channel FROM transcripts WHERE meeting_id = ? ORDER BY audio_start_time",
    )
    .bind(&meeting_id)
    .fetch_all(pool)
    .await
    .expect("rows");
    assert_eq!(rows.len(), 2, "one row became two parts");
    assert_eq!(rows[0].1.as_deref(), Some("microphone"));
    assert_eq!(rows[1].1.as_deref(), Some("system"));

    let keys = aligned_keys(pool, &meeting_id, &turns).await;
    assert_eq!(
        keys,
        vec![LOCAL_SPEAKER_KEY.to_string(), "spk_0".to_string()],
        "owner part labels {LOCAL_SPEAKER_KEY}, system part labels spk_0"
    );
}

#[tokio::test]
async fn overridden_glued_row_stays_whole_regardless_of_split_direction() {
    let (_dir, db) = fresh_db().await;
    let pool = db.pool();
    let (meeting_id, row_id) = seed_meeting_with_row(pool, MERGED_TEXT, 0.0, 6.7).await;
    tag_channel(pool, &row_id, "microphone").await;

    // The owner hand-corrected this line: its id <-> span meaning must stay
    // stable even though it straddles an owner<->remote handoff, so the
    // splitter must leave it whole (specs/0019 WS2.3).
    let applied =
        TranscriptSpeakerOverridesRepository::set(pool, &meeting_id, &row_id, LOCAL_SPEAKER_KEY)
            .await
            .expect("set override");
    assert!(applied);

    let turns = vec![turn(0.0, 3.0, LOCAL_SPEAKER_KEY), turn(3.4, 6.3, "spk_0")];
    let n = split_straddling_rows(pool, &meeting_id, &turns)
        .await
        .expect("split");
    assert_eq!(
        n, 0,
        "an overridden row must never split, even straddling owner<->remote"
    );

    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM transcripts WHERE meeting_id = ?")
        .bind(&meeting_id)
        .fetch_one(pool)
        .await
        .expect("count");
    assert_eq!(count, 1, "the overridden row stays a single row");
}
