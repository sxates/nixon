//! Bounded transcript excerpts — the evidence windows Ask AI's gather stage
//! appends to a summary/notes doc (specs/0035 transcript-excerpt policy,
//! extended by specs/0056 W3 with speaker names and multi-hit windows).
//!
//! Split out of `transcript.rs` (allowlisted under the specs/0042 ratchet).
//! Everything here is an `impl TranscriptsRepository` block, so callers keep
//! using `TranscriptsRepository::…`.

use super::transcript::TranscriptsRepository;
use sqlx::{Error as SqlxError, SqlitePool};

/// The line written between two non-contiguous windows of a merged excerpt
/// (see [`TranscriptsRepository::get_transcript_excerpts_with_speakers`]).
pub const EXCERPT_GAP_MARKER: &str = "[…]";

impl TranscriptsRepository {
    /// Loads a bounded transcript excerpt: ±`window` segments around
    /// `transcript_id`, in the same canonical recording order as
    /// [`Self::get_full_transcript`] (specs/0035 transcript-excerpt policy —
    /// the evidence window when a question's best FTS hit is transcript-only).
    ///
    /// Returns `None` when the segment id no longer exists (best-effort:
    /// "Transcribe now" regenerates segment ids, same caveat as search's
    /// deep-link `transcript_id`).
    ///
    /// Windowed in SQL: only the ±`window` rows' text leaves the database, not
    /// the whole meeting (this sits on the Ask-AI preview's debounce path, and
    /// long meetings run to megabytes). The hit position and the window are
    /// computed inside ONE query over one `ROW_NUMBER()` pass, so tie-breaking
    /// between equal `(audio_start_time, timestamp)` pairs cannot diverge the
    /// way a separate find-then-fetch pair of queries could.
    pub async fn get_transcript_excerpt(
        pool: &SqlitePool,
        meeting_id: &str,
        transcript_id: &str,
        window: usize,
    ) -> Result<Option<String>, SqlxError> {
        let rows = sqlx::query_as::<_, (String,)>(
            "WITH ordered AS (
                 SELECT id,
                        ROW_NUMBER() OVER (
                            ORDER BY audio_start_time IS NULL, audio_start_time, timestamp
                        ) AS rn
                 FROM transcripts
                 WHERE meeting_id = ?
             ),
             hit AS (SELECT rn FROM ordered WHERE id = ?)
             SELECT t.transcript
             FROM ordered o
             CROSS JOIN hit h
             JOIN transcripts t ON t.id = o.id
             WHERE o.rn BETWEEN h.rn - ? AND h.rn + ?
             ORDER BY o.rn",
        )
        .bind(meeting_id)
        .bind(transcript_id)
        .bind(window as i64)
        .bind(window as i64)
        .fetch_all(pool)
        .await?;

        // The hit row is always inside its own window, so zero rows means the
        // segment id wasn't found (or belongs to another meeting).
        if rows.is_empty() {
            return Ok(None);
        }

        let excerpt = rows
            .iter()
            .map(|(transcript,)| transcript.trim())
            .filter(|transcript| !transcript.is_empty())
            .collect::<Vec<_>>()
            .join("\n");

        Ok(Some(excerpt))
    }

    /// Loads the speaker-labelled evidence windows around several transcript
    /// hits at once (specs/0056 W3 — the excerpt Ask AI's gather stage appends
    /// when the question hit the transcript but the doc body is the summary or
    /// notes, whichever source ranked best).
    ///
    /// For each id in `hit_ids` the ±`window` segments by the canonical
    /// recording order (`audio_start_time IS NULL, audio_start_time,
    /// timestamp` — the same order as [`Self::get_full_transcript`]) are
    /// selected; windows that overlap or touch merge into one contiguous
    /// block, and non-contiguous blocks are separated by an
    /// [`EXCERPT_GAP_MARKER`] line. Every line is `Name: text` when the
    /// segment's `speaker` key resolves through the `speakers` table (same
    /// `LEFT JOIN` as [`Self::get_transcript_segments_with_speakers`]), bare
    /// `text` otherwise. Blank segments are dropped.
    ///
    /// Returns `None` when `hit_ids` is empty or none of the ids exist for
    /// this meeting (best-effort: "Transcribe now" regenerates segment ids).
    /// Ids that no longer exist are simply ignored when at least one does.
    ///
    /// Windowed in SQL, one `ROW_NUMBER()` pass, like
    /// [`Self::get_transcript_excerpt`]: only the windows' rows leave the
    /// database, never the whole meeting.
    pub async fn get_transcript_excerpts_with_speakers(
        pool: &SqlitePool,
        meeting_id: &str,
        hit_ids: &[String],
        window: usize,
    ) -> Result<Option<String>, SqlxError> {
        if hit_ids.is_empty() {
            return Ok(None);
        }
        let placeholders = vec!["?"; hit_ids.len()].join(", ");
        let sql = format!(
            "WITH ordered AS (
                 SELECT id,
                        ROW_NUMBER() OVER (
                            ORDER BY audio_start_time IS NULL, audio_start_time, timestamp
                        ) AS rn
                 FROM transcripts
                 WHERE meeting_id = ?
             ),
             hits AS (SELECT rn FROM ordered WHERE id IN ({placeholders}))
             SELECT o.rn, s.display_name, t.transcript
             FROM ordered o
             JOIN transcripts t ON t.id = o.id
             LEFT JOIN speakers s
               ON s.meeting_id = t.meeting_id AND s.speaker_key = t.speaker
             WHERE EXISTS (SELECT 1 FROM hits h WHERE o.rn BETWEEN h.rn - ? AND h.rn + ?)
             ORDER BY o.rn"
        );

        let mut query = sqlx::query_as::<_, (i64, Option<String>, String)>(&sql).bind(meeting_id);
        for id in hit_ids {
            query = query.bind(id);
        }
        let rows = query
            .bind(window as i64)
            .bind(window as i64)
            .fetch_all(pool)
            .await?;

        // Every existing hit sits inside its own window, so zero rows means
        // none of the ids exist (or they belong to another meeting).
        if rows.is_empty() {
            return Ok(None);
        }
        Ok(Some(render_windows(&rows)))
    }
}

/// Formats `(row_number, speaker_name, text)` rows — already in recording
/// order — as excerpt lines: a gap in the row numbers becomes one
/// [`EXCERPT_GAP_MARKER`] line (emitted lazily, so blank segments at a window's
/// edge never leave a dangling marker), a resolved speaker prefixes its line.
fn render_windows(rows: &[(i64, Option<String>, String)]) -> String {
    let mut lines: Vec<String> = Vec::with_capacity(rows.len());
    let mut prev_rn: Option<i64> = None;
    let mut pending_gap = false;

    for (rn, speaker, text) in rows {
        if prev_rn.is_some_and(|prev| *rn > prev + 1) {
            pending_gap = true;
        }
        prev_rn = Some(*rn);

        let text = text.trim();
        if text.is_empty() {
            continue;
        }
        if pending_gap && !lines.is_empty() {
            lines.push(EXCERPT_GAP_MARKER.to_string());
        }
        pending_gap = false;

        match speaker.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            Some(name) => lines.push(format!("{name}: {text}")),
            None => lines.push(text.to_string()),
        }
    }

    lines.join("\n")
}

/// Fixture test for the windowed `get_transcript_excerpt` query: it must
/// produce byte-identical output to the previous load-everything-then-slice
/// implementation (kept here as the in-test reference) across hit positions,
/// window sizes, NULL `audio_start_time` ordering, and blank segments.
#[cfg(test)]
mod excerpt_tests {
    use super::TranscriptsRepository;
    use crate::database::repositories::meeting::MeetingsRepository;
    use sqlx::sqlite::SqlitePoolOptions;
    use sqlx::SqlitePool;

    /// Fresh in-memory SQLite through the app's real migration set (mirrors
    /// `search.rs`'s test pool). One connection max — each in-memory
    /// connection is a separate database.
    async fn test_pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("open in-memory sqlite");
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .expect("migrations");
        pool
    }

    async fn insert_segment(
        pool: &SqlitePool,
        meeting_id: &str,
        id: &str,
        text: &str,
        audio_start_time: Option<f64>,
        timestamp: &str,
    ) {
        sqlx::query(
            "INSERT INTO transcripts (id, meeting_id, transcript, timestamp, audio_start_time) \
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(id)
        .bind(meeting_id)
        .bind(text)
        .bind(timestamp)
        .bind(audio_start_time)
        .execute(pool)
        .await
        .expect("insert transcript segment");
    }

    /// The pre-windowed-query implementation, verbatim: load every row in the
    /// canonical order, find the hit, slice ±window in Rust.
    async fn reference_excerpt(
        pool: &SqlitePool,
        meeting_id: &str,
        transcript_id: &str,
        window: usize,
    ) -> Option<String> {
        let rows = sqlx::query_as::<_, (String, String)>(
            "SELECT id, transcript
             FROM transcripts
             WHERE meeting_id = ?
             ORDER BY audio_start_time IS NULL, audio_start_time, timestamp",
        )
        .bind(meeting_id)
        .fetch_all(pool)
        .await
        .expect("reference query");

        let hit = rows.iter().position(|(id, _)| id == transcript_id)?;
        let start = hit.saturating_sub(window);
        let end = (hit + window + 1).min(rows.len());
        Some(
            rows[start..end]
                .iter()
                .map(|(_, transcript)| transcript.trim())
                .filter(|transcript| !transcript.is_empty())
                .collect::<Vec<_>>()
                .join("\n"),
        )
    }

    #[tokio::test]
    async fn windowed_excerpt_matches_the_reference_implementation() {
        let pool = test_pool().await;
        let meeting_id = MeetingsRepository::create_meeting(
            &pool,
            Some("Fixture".into()),
            None,
            None,
            None,
            None,
        )
        .await
        .expect("create_meeting");

        // 8 segments: timed rows (distinct audio_start_time), then untimed
        // rows (NULL audio_start_time, ordered by timestamp — the pre-column
        // legacy shape), with a blank segment mixed in. Distinct ordering keys
        // throughout so the canonical order is total (ties are unordered in
        // SQL and would make any implementation nondeterministic).
        let fixture: &[(&str, &str, Option<f64>, &str)] = &[
            (
                "seg-a",
                "alpha opening remarks",
                Some(0.0),
                "2026-07-01T10:00:00Z",
            ),
            (
                "seg-b",
                "beta agenda review",
                Some(2.0),
                "2026-07-01T10:00:02Z",
            ),
            ("seg-c", "   ", Some(4.0), "2026-07-01T10:00:04Z"), // blank: dropped from output
            (
                "seg-d",
                "delta the key decision",
                Some(6.0),
                "2026-07-01T10:00:06Z",
            ),
            (
                "seg-e",
                "epsilon follow-ups",
                Some(8.0),
                "2026-07-01T10:00:08Z",
            ),
            (
                "seg-f",
                "zeta untimed legacy row",
                None,
                "2026-07-01T10:00:10Z",
            ),
            (
                "seg-g",
                "eta another untimed row",
                None,
                "2026-07-01T10:00:12Z",
            ),
            ("seg-h", "theta closing", None, "2026-07-01T10:00:14Z"),
        ];
        for (id, text, ast, ts) in fixture {
            insert_segment(&pool, &meeting_id, id, text, *ast, ts).await;
        }

        for (id, _, _, _) in fixture {
            for window in [0usize, 1, 2, 3, 15] {
                let windowed =
                    TranscriptsRepository::get_transcript_excerpt(&pool, &meeting_id, id, window)
                        .await
                        .expect("windowed query");
                let reference = reference_excerpt(&pool, &meeting_id, id, window).await;
                assert_eq!(
                    windowed, reference,
                    "windowed != reference for hit {id}, window {window}"
                );
            }
        }

        // Spot-check the shape: hit in the middle, window 1 spans neighbors.
        let excerpt = TranscriptsRepository::get_transcript_excerpt(&pool, &meeting_id, "seg-d", 1)
            .await
            .unwrap()
            .expect("hit exists");
        assert_eq!(excerpt, "delta the key decision\nepsilon follow-ups");

        // Unknown / cross-meeting ids stay None.
        assert_eq!(
            TranscriptsRepository::get_transcript_excerpt(&pool, &meeting_id, "seg-zz", 3)
                .await
                .unwrap(),
            None
        );
        assert_eq!(
            TranscriptsRepository::get_transcript_excerpt(&pool, "other-meeting", "seg-a", 3)
                .await
                .unwrap(),
            None
        );
    }
}

/// specs/0056 W3: the multi-hit, speaker-labelled excerpt.
#[cfg(test)]
mod speaker_excerpt_tests {
    use super::{TranscriptsRepository, EXCERPT_GAP_MARKER};
    use crate::database::repositories::meeting::MeetingsRepository;
    use crate::database::repositories::speaker::SpeakersRepository;
    use sqlx::sqlite::SqlitePoolOptions;
    use sqlx::SqlitePool;

    async fn test_pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("open in-memory sqlite");
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .expect("migrations");
        pool
    }

    /// `n` timed segments `{prefix}-000…{prefix}-{n-1}` with text `line <i>`,
    /// every other one tagged `spk_0` (the rest untagged, like an un-diarized
    /// row). `prefix` keeps ids unique across meetings (`transcripts.id` is
    /// the primary key).
    async fn meeting_with_segments(pool: &SqlitePool, n: usize, prefix: &str) -> String {
        let meeting_id = MeetingsRepository::create_meeting(
            pool,
            Some("Fixture".into()),
            None,
            None,
            None,
            None,
        )
        .await
        .expect("create_meeting");
        for i in 0..n {
            let speaker = if i % 2 == 0 { Some("spk_0") } else { None };
            sqlx::query(
                "INSERT INTO transcripts (id, meeting_id, transcript, timestamp, audio_start_time, speaker) \
                 VALUES (?, ?, ?, ?, ?, ?)",
            )
            .bind(format!("{prefix}-{i:03}"))
            .bind(&meeting_id)
            .bind(format!("line {i}"))
            .bind(format!("2026-07-01T10:{:02}:{:02}Z", i / 60, i % 60))
            .bind(i as f64 * 2.0)
            .bind(speaker)
            .execute(pool)
            .await
            .expect("insert transcript segment");
        }
        meeting_id
    }

    fn ids(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[tokio::test]
    async fn overlapping_windows_merge_into_one_block_without_a_gap_marker() {
        let pool = test_pool().await;
        let m = meeting_with_segments(&pool, 200, "seg").await;

        // Hits 5 apart, window 15: [35..65] ∪ [40..70] = one block [35..70].
        let excerpt = TranscriptsRepository::get_transcript_excerpts_with_speakers(
            &pool,
            &m,
            &ids(&["seg-050", "seg-055"]),
            15,
        )
        .await
        .unwrap()
        .expect("hits exist");

        let lines: Vec<&str> = excerpt.lines().collect();
        assert_eq!(lines.len(), 36, "35..=70 inclusive, got:\n{excerpt}");
        assert!(lines[0].ends_with("line 35"), "first line: {}", lines[0]);
        assert!(lines[35].ends_with("line 70"), "last line: {}", lines[35]);
        assert!(
            !excerpt.contains(EXCERPT_GAP_MARKER),
            "one contiguous block has no gap marker:\n{excerpt}"
        );
        // Recording order, not hit order: passing the hits reversed changes nothing.
        let reversed = TranscriptsRepository::get_transcript_excerpts_with_speakers(
            &pool,
            &m,
            &ids(&["seg-055", "seg-050"]),
            15,
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(reversed, excerpt);
    }

    #[tokio::test]
    async fn distant_windows_stay_separate_blocks_joined_by_a_gap_marker() {
        let pool = test_pool().await;
        let m = meeting_with_segments(&pool, 200, "seg").await;

        // Hits 100 apart, window 15: [35..65] and [135..165].
        let excerpt = TranscriptsRepository::get_transcript_excerpts_with_speakers(
            &pool,
            &m,
            &ids(&["seg-050", "seg-150"]),
            15,
        )
        .await
        .unwrap()
        .expect("hits exist");

        let blocks: Vec<&str> = excerpt.split(EXCERPT_GAP_MARKER).collect();
        assert_eq!(blocks.len(), 2, "exactly one gap marker:\n{excerpt}");
        let first: Vec<&str> = blocks[0].trim().lines().collect();
        let second: Vec<&str> = blocks[1].trim().lines().collect();
        assert_eq!(first.len(), 31, "35..=65:\n{}", blocks[0]);
        assert_eq!(second.len(), 31, "135..=165:\n{}", blocks[1]);
        assert!(first.last().unwrap().ends_with("line 65"));
        assert!(second[0].ends_with("line 135"));
        // The marker sits on its own line.
        assert!(excerpt.contains(&format!("line 65\n{EXCERPT_GAP_MARKER}\n")));
    }

    #[tokio::test]
    async fn touching_windows_merge_without_a_gap_marker() {
        let pool = test_pool().await;
        let m = meeting_with_segments(&pool, 100, "seg").await;

        // [10..30] and [31..51]: adjacent rows, nothing skipped → no marker.
        let excerpt = TranscriptsRepository::get_transcript_excerpts_with_speakers(
            &pool,
            &m,
            &ids(&["seg-020", "seg-041"]),
            10,
        )
        .await
        .unwrap()
        .expect("hits exist");
        assert!(!excerpt.contains(EXCERPT_GAP_MARKER), "{excerpt}");
        assert_eq!(excerpt.lines().count(), 42);

        // [10..30] and [32..52]: one row (31) skipped → marker.
        let excerpt = TranscriptsRepository::get_transcript_excerpts_with_speakers(
            &pool,
            &m,
            &ids(&["seg-020", "seg-042"]),
            10,
        )
        .await
        .unwrap()
        .expect("hits exist");
        assert!(excerpt.contains(EXCERPT_GAP_MARKER), "{excerpt}");
    }

    #[tokio::test]
    async fn lines_are_prefixed_with_the_resolved_speaker_name_only() {
        let pool = test_pool().await;
        let m = meeting_with_segments(&pool, 10, "seg").await;
        // spk_0 resolves to "Priya"; the untagged rows have no speakers row.
        SpeakersRepository::upsert(&pool, &m, "spk_0", "Priya", false, None, None, None)
            .await
            .expect("speaker row");

        let excerpt = TranscriptsRepository::get_transcript_excerpts_with_speakers(
            &pool,
            &m,
            &ids(&["seg-004"]),
            1,
        )
        .await
        .unwrap()
        .expect("hit exists");
        assert_eq!(excerpt, "line 3\nPriya: line 4\nline 5");

        // A tagged segment whose key has NO speakers row stays bare (the join
        // is LEFT, and an unresolved key is not a name).
        let m2 = meeting_with_segments(&pool, 3, "m2").await;
        let excerpt = TranscriptsRepository::get_transcript_excerpts_with_speakers(
            &pool,
            &m2,
            &ids(&["m2-000"]),
            0,
        )
        .await
        .unwrap()
        .expect("hit exists");
        assert_eq!(excerpt, "line 0");
    }

    #[tokio::test]
    async fn unknown_ids_and_empty_input_yield_none() {
        let pool = test_pool().await;
        let m = meeting_with_segments(&pool, 5, "seg").await;

        assert_eq!(
            TranscriptsRepository::get_transcript_excerpts_with_speakers(&pool, &m, &[], 15)
                .await
                .unwrap(),
            None,
            "no hit ids → nothing to excerpt"
        );
        assert_eq!(
            TranscriptsRepository::get_transcript_excerpts_with_speakers(
                &pool,
                &m,
                &ids(&["seg-zz", "seg-yy"]),
                15
            )
            .await
            .unwrap(),
            None,
            "ids that no longer exist → None"
        );
        assert_eq!(
            TranscriptsRepository::get_transcript_excerpts_with_speakers(
                &pool,
                "other-meeting",
                &ids(&["seg-000"]),
                15
            )
            .await
            .unwrap(),
            None,
            "an id from another meeting → None"
        );
        // A mix of one live and one dead id still excerpts the live one.
        let excerpt = TranscriptsRepository::get_transcript_excerpts_with_speakers(
            &pool,
            &m,
            &ids(&["seg-zz", "seg-002"]),
            0,
        )
        .await
        .unwrap();
        assert_eq!(excerpt.as_deref(), Some("line 2"));
    }
}
