use crate::database::models::DateTimeUtc;
use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::{Error as SqlxError, FromRow, SqlitePool};
use tracing::info;
use uuid::Uuid;

use super::{normalize_title, MeetingsRepository};

/// One meeting manually linked into a series (a `meeting_series_links` row joined to its
/// meeting), for the Prep tab's "Linked meetings" list (specs/0041 WS4). Serialized
/// camelCase for the frontend.
#[derive(Debug, Clone, FromRow, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SeriesLinkedMeeting {
    pub id: String,
    pub title: String,
    /// From `meetings.created_at` — the meeting's effective start / displayed date.
    pub started_at: DateTimeUtc,
}

impl MeetingsRepository {
    /// Sets (or clears) the recurring-series key on a meeting (specs/0036). Called after
    /// create for a calendar-linked meeting once the event's `external_id` is known. Kept
    /// separate from [`create_meeting`] so that method's signature (and its ~80 call sites)
    /// stays untouched.
    pub async fn set_calendar_series_key(
        pool: &SqlitePool,
        meeting_id: &str,
        series_key: Option<&str>,
    ) -> Result<(), SqlxError> {
        let series_key = series_key.map(str::trim).filter(|k| !k.is_empty());
        sqlx::query("UPDATE meetings SET calendar_series_key = ? WHERE id = ?")
            .bind(series_key)
            .bind(meeting_id)
            .execute(pool)
            .await?;
        Ok(())
    }

    /// Prior COMPLETED occurrences of the same recurring series (specs/0036, additive union
    /// per specs/0041 WS4), newest first, limited to those that have content to summarize
    /// (a completed summary, else a transcript, else non-empty notes) so a brief never pins
    /// a doc-less meeting.
    ///
    /// The match is a UNION of three arms (a prior qualifies if ANY hits — the old
    /// series-key-shadows-title `else if` is gone):
    ///   1. series key: `meetings.calendar_series_key = key`,
    ///   2. manual link: a `meeting_series_links` row with the same key (specs/0041 WS4 —
    ///      ad-hoc recordings the user pinned into this series),
    ///   3. normalized title: `LOWER(TRIM(title))` equality, only when `title` is non-blank.
    ///      When a series key IS passed, this arm only pulls in *unaffiliated* priors
    ///      (NULL `calendar_series_key` and no link to a different key) — two distinct
    ///      recurring series that share a title (e.g. two "Weekly 1:1"s) must never bleed
    ///      into each other's prep briefs. A keyless lookup (pure title fallback) keeps
    ///      the unrestricted behavior and matches keyed priors too.
    ///
    /// All arms still require `origin = 'recorded'` and `created_at` strictly before `before`
    /// (the target occurrence's start) — `scheduled` rows (future/empty prep placeholders)
    /// are excluded. Deduped by meeting id (the links join is at most 1:1 per meeting, so
    /// one output row per meeting), newest first, capped at `limit`.
    ///
    /// Callers with a target meeting row should pass the EFFECTIVE key from
    /// [`Self::resolve_effective_series_key`] so a manually-keyed (`manual:{uuid}`) target
    /// matches its linked priors.
    pub async fn find_prior_series_occurrences(
        pool: &SqlitePool,
        series_key: Option<&str>,
        title: &str,
        before: DateTime<Utc>,
        limit: i64,
    ) -> Result<Vec<String>, SqlxError> {
        // A meeting "has content" if it has a completed summary, any transcript, or notes.
        const HAS_CONTENT: &str = "( \
            EXISTS (SELECT 1 FROM summary_processes sp WHERE sp.meeting_id = m.id AND sp.status = 'completed') \
            OR EXISTS (SELECT 1 FROM transcripts t WHERE t.meeting_id = m.id) \
            OR EXISTS (SELECT 1 FROM meeting_notes n WHERE n.meeting_id = m.id AND TRIM(COALESCE(n.notes_markdown, '')) != '') \
        )";

        let key = series_key.map(str::trim).filter(|k| !k.is_empty());
        if key.is_none() && normalize_title(title).is_empty() {
            return Ok(Vec::new()); // no usable arm — a blank key/title must never match
        }

        // Title folding stays SQL-side on BOTH operands (SQLite's ASCII-only LOWER must be
        // applied identically to the column and the parameter — see suggest_template_for_title).
        // The `TRIM(?3) <> ''` guard keeps a blank title from matching blank-titled rows.
        // With a series key (?1 NOT NULL) the title arm additionally requires the prior to
        // be unaffiliated — NULL calendar_series_key and no link row pointing at a
        // DIFFERENT series — so same-titled distinct series never cross-match; a keyless
        // lookup keeps the unrestricted title match (keyed priors included).
        let rows: Vec<(String,)> = sqlx::query_as(&format!(
            "SELECT m.id FROM meetings m \
             LEFT JOIN meeting_series_links l ON l.meeting_id = m.id \
             WHERE m.origin = 'recorded' AND m.created_at < ?2 \
               AND ( (?1 IS NOT NULL AND (m.calendar_series_key = ?1 OR l.series_key = ?1)) \
                  OR ( TRIM(?3) <> '' AND LOWER(TRIM(m.title)) = LOWER(TRIM(?3)) \
                       AND (?1 IS NULL \
                            OR (m.calendar_series_key IS NULL \
                                AND (l.series_key IS NULL OR l.series_key = ?1))) ) ) \
               AND {HAS_CONTENT} \
             ORDER BY m.created_at DESC LIMIT ?4"
        ))
        .bind(key)
        .bind(before)
        .bind(title)
        .bind(limit)
        .fetch_all(pool)
        .await?;
        Ok(rows.into_iter().map(|(id,)| id).collect())
    }

    /// The EFFECTIVE series key for one meeting (specs/0041 WS4). Resolution order:
    ///   1. `meetings.calendar_series_key` — the calendar-stamped key (authoritative),
    ///   2. `meeting_series_links.series_key` — a manual association (possibly a minted
    ///      `manual:{uuid}` key when neither side had a calendar one).
    ///
    /// `None` when the meeting has neither (or doesn't exist). Callers pass this to
    /// [`Self::find_prior_series_occurrences`] so manually-keyed targets match their priors.
    pub async fn resolve_effective_series_key(
        pool: &SqlitePool,
        meeting_id: &str,
    ) -> Result<Option<String>, SqlxError> {
        let key: Option<Option<String>> = sqlx::query_scalar(
            "SELECT COALESCE(m.calendar_series_key, l.series_key) \
             FROM meetings m \
             LEFT JOIN meeting_series_links l ON l.meeting_id = m.id \
             WHERE m.id = ?",
        )
        .bind(meeting_id)
        .fetch_optional(pool)
        .await?;
        Ok(key
            .flatten()
            .map(|k| k.trim().to_string())
            .filter(|k| !k.is_empty()))
    }

    /// Manually pin `meeting_id` into the series of `target_meeting_id` (specs/0041 WS4 —
    /// the "Link previous meeting…" action). Returns the series key both rows now share.
    ///
    /// Key resolution order (documented contract, mirrors
    /// [`Self::resolve_effective_series_key`]):
    ///   1. the target's `meetings.calendar_series_key` (calendar-stamped),
    ///   2. the target's existing `meeting_series_links.series_key`,
    ///   3. neither → mint a synthetic `manual:{uuid}` key and stamp it on BOTH rows via
    ///      `meeting_series_links` — we NEVER mutate `meetings.calendar_series_key` itself
    ///      (that column stays calendar-owned).
    ///
    /// Re-linking an already-linked meeting moves it to the new series (PK upsert).
    pub async fn link_meeting_to_series(
        pool: &SqlitePool,
        meeting_id: &str,
        target_meeting_id: &str,
    ) -> Result<String, SqlxError> {
        let meeting_id = meeting_id.trim();
        let target_meeting_id = target_meeting_id.trim();
        if meeting_id.is_empty() || target_meeting_id.is_empty() {
            return Err(SqlxError::Protocol(
                "meeting_id and target_meeting_id cannot be empty".to_string(),
            ));
        }
        if meeting_id == target_meeting_id {
            return Err(SqlxError::Protocol(
                "A meeting cannot be linked to its own series".to_string(),
            ));
        }
        // Both rows must exist (the links FK only guards meeting_id; check the target too,
        // and give link_meeting a clear "not found" instead of an FK violation).
        for id in [meeting_id, target_meeting_id] {
            let exists: Option<(i64,)> = sqlx::query_as("SELECT 1 FROM meetings WHERE id = ?")
                .bind(id)
                .fetch_optional(pool)
                .await?;
            if exists.is_none() {
                return Err(SqlxError::Protocol(format!("Meeting {id} not found")));
            }
        }

        let upsert = |id: String, key: String| async move {
            sqlx::query(
                "INSERT INTO meeting_series_links (meeting_id, series_key) VALUES (?, ?) \
                 ON CONFLICT(meeting_id) DO UPDATE SET series_key = excluded.series_key",
            )
            .bind(id)
            .bind(key)
            .execute(pool)
            .await
            .map(|_| ())
        };

        match Self::resolve_effective_series_key(pool, target_meeting_id).await? {
            // 1./2. The target already has a key (calendar or link) — stamp only the
            // meeting being linked in.
            Some(key) => {
                upsert(meeting_id.to_string(), key.clone()).await?;
                info!("linked meeting {meeting_id} into series {key}");
                Ok(key)
            }
            // 3. Mint a manual key and stamp BOTH rows via the links table.
            None => {
                let key = format!("manual:{}", Uuid::new_v4());
                upsert(target_meeting_id.to_string(), key.clone()).await?;
                upsert(meeting_id.to_string(), key.clone()).await?;
                info!(
                    "linked meetings {meeting_id} + {target_meeting_id} under minted series {key}"
                );
                Ok(key)
            }
        }
    }

    /// The manual series link for one meeting (`meeting_series_links.series_key`), if any.
    pub async fn get_series_link(
        pool: &SqlitePool,
        meeting_id: &str,
    ) -> Result<Option<String>, SqlxError> {
        sqlx::query_scalar("SELECT series_key FROM meeting_series_links WHERE meeting_id = ?")
            .bind(meeting_id)
            .fetch_optional(pool)
            .await
    }

    /// Remove `meeting_id`'s manual series link (specs/0041 WS4 unlink affordance). Only the
    /// `meeting_series_links` row is touched — a calendar-stamped
    /// `meetings.calendar_series_key` is never cleared here. Returns whether a link existed.
    pub async fn unlink_meeting_from_series(
        pool: &SqlitePool,
        meeting_id: &str,
    ) -> Result<bool, SqlxError> {
        let result = sqlx::query("DELETE FROM meeting_series_links WHERE meeting_id = ?")
            .bind(meeting_id)
            .execute(pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Meetings manually linked into `series_key` (rows in `meeting_series_links`), newest
    /// first, excluding `exclude_meeting_id` (the Prep tab's own target). Backs the Prep
    /// tab's "Linked meetings" list with its unlink affordance (specs/0041 WS4).
    pub async fn linked_meetings_for_series(
        pool: &SqlitePool,
        series_key: &str,
        exclude_meeting_id: &str,
    ) -> Result<Vec<SeriesLinkedMeeting>, SqlxError> {
        sqlx::query_as::<_, SeriesLinkedMeeting>(
            "SELECT m.id AS id, m.title AS title, m.created_at AS started_at \
             FROM meeting_series_links l \
             JOIN meetings m ON m.id = l.meeting_id \
             WHERE l.series_key = ?1 AND m.id <> ?2 \
             ORDER BY m.created_at DESC",
        )
        .bind(series_key)
        .bind(exclude_meeting_id)
        .fetch_all(pool)
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::repositories::meeting::test_support::{
        dt, memory_db, recorded_with_summary,
    };

    // ---- specs/0036: series detection + scheduled origin + adoption ----

    /// specs/0041 WS4: the matcher is a UNION — series key OR normalized title OR manual
    /// link — instead of the old series-key-shadows-title `else if`.
    #[tokio::test]
    async fn prior_series_occurrences_union_of_series_key_and_title() {
        let pool = memory_db().await;

        // Two calendar occurrences of series-A, one DIFFERENTLY-TITLED occurrence of the
        // same series (renamed recurring meeting), one same-title ad-hoc recording with NO
        // series key (the WS4 acceptance case), and one different-title/different-key decoy.
        let o1 = recorded_with_summary(
            &pool,
            "Design Review",
            "2026-06-01T10:00:00Z",
            Some("series-A"),
        )
        .await;
        let o2 = recorded_with_summary(
            &pool,
            "Design Review",
            "2026-06-08T10:00:00Z",
            Some("series-A"),
        )
        .await;
        let renamed = recorded_with_summary(
            &pool,
            "Design Review (rescheduled)",
            "2026-06-12T10:00:00Z",
            Some("series-A"),
        )
        .await;
        let adhoc =
            recorded_with_summary(&pool, "  design REVIEW ", "2026-06-15T10:00:00Z", None).await;
        let decoy =
            recorded_with_summary(&pool, "Retro", "2026-06-14T10:00:00Z", Some("series-B")).await;

        // Union: key matches (incl. the renamed occurrence) AND the same-title ad-hoc row,
        // newest first; the decoy matches neither arm.
        let prior = MeetingsRepository::find_prior_series_occurrences(
            &pool,
            Some("series-A"),
            "Design Review",
            dt("2026-06-22T10:00:00Z"),
            10,
        )
        .await
        .unwrap();
        assert_eq!(
            prior,
            vec![adhoc.clone(), renamed.clone(), o2.clone(), o1.clone()],
            "series-key ∪ title, newest first, decoy excluded"
        );

        // Same LIMIT semantics: the cap applies to the merged set.
        let capped = MeetingsRepository::find_prior_series_occurrences(
            &pool,
            Some("series-A"),
            "Design Review",
            dt("2026-06-22T10:00:00Z"),
            2,
        )
        .await
        .unwrap();
        assert_eq!(capped, vec![adhoc.clone(), renamed]);

        // before-cutoff is strict on EVERY arm: a target between o1 and o2 only sees o1.
        let prior_mid = MeetingsRepository::find_prior_series_occurrences(
            &pool,
            Some("series-A"),
            "Design Review",
            dt("2026-06-05T00:00:00Z"),
            10,
        )
        .await
        .unwrap();
        assert_eq!(prior_mid, vec![o1.clone()]);

        // No series key → the title arm alone (ad-hoc + the two same-titled A-occurrences).
        let by_title = MeetingsRepository::find_prior_series_occurrences(
            &pool,
            None,
            "  design review  ", // normalized match
            dt("2026-06-22T10:00:00Z"),
            10,
        )
        .await
        .unwrap();
        assert_eq!(by_title, vec![adhoc, o2, o1]);

        // Blank key AND blank title → no arm, no matches (never "match everything").
        let blank = MeetingsRepository::find_prior_series_occurrences(
            &pool,
            Some("  "),
            "   ",
            dt("2026-06-22T10:00:00Z"),
            10,
        )
        .await
        .unwrap();
        assert!(blank.is_empty());
        let _ = decoy;
    }

    /// Code-review fix (specs/0041 WS4): the title arm must never pull a member of a
    /// DIFFERENT series into a keyed lookup — two distinct recurring series that share a
    /// title (two "Weekly 1:1"s) previously bled into each other's prep briefs. With a
    /// key, title only matches *unaffiliated* (ad-hoc) priors; a keyless lookup keeps the
    /// old unrestricted title behavior.
    #[tokio::test]
    async fn title_arm_never_pulls_in_a_different_series() {
        let pool = memory_db().await;

        // (a) A same-titled occurrence of ANOTHER calendar series.
        let other_series = recorded_with_summary(
            &pool,
            "Weekly 1:1",
            "2026-06-01T10:00:00Z",
            Some("series-B"),
        )
        .await;
        // Our own keyed occurrence.
        let ours = recorded_with_summary(
            &pool,
            "Weekly 1:1",
            "2026-06-02T10:00:00Z",
            Some("series-A"),
        )
        .await;
        // (b) Same-title unaffiliated ad-hoc recording (the WS4 owner case).
        let adhoc = recorded_with_summary(&pool, "Weekly 1:1", "2026-06-03T10:00:00Z", None).await;
        // Same-title ad-hoc, but manually linked to the OTHER series → affiliated, excluded.
        let linked_other =
            recorded_with_summary(&pool, "Weekly 1:1", "2026-06-04T10:00:00Z", None).await;
        sqlx::query(
            "INSERT INTO meeting_series_links (meeting_id, series_key) VALUES (?, 'series-B')",
        )
        .bind(&linked_other)
        .execute(&pool)
        .await
        .unwrap();
        // (d) Manually linked to THIS series + the title coincidence → matches exactly once.
        let linked_ours =
            recorded_with_summary(&pool, "Weekly 1:1", "2026-06-05T10:00:00Z", None).await;
        sqlx::query(
            "INSERT INTO meeting_series_links (meeting_id, series_key) VALUES (?, 'series-A')",
        )
        .bind(&linked_ours)
        .execute(&pool)
        .await
        .unwrap();

        // Keyed lookup: series-B members (calendar-keyed or manually linked) never bleed
        // in via the shared title; the unaffiliated ad-hoc and our own members do.
        let prior = MeetingsRepository::find_prior_series_occurrences(
            &pool,
            Some("series-A"),
            "Weekly 1:1",
            dt("2026-06-22T10:00:00Z"),
            10,
        )
        .await
        .unwrap();
        assert_eq!(
            prior,
            vec![linked_ours.clone(), adhoc.clone(), ours.clone()],
            "no cross-series bleed; multi-arm hits deduped; newest first"
        );

        // (c) Keyless lookup (pure title fallback) keeps the OLD unrestricted behavior:
        // keyed and linked priors still match by title.
        let by_title = MeetingsRepository::find_prior_series_occurrences(
            &pool,
            None,
            "Weekly 1:1",
            dt("2026-06-22T10:00:00Z"),
            10,
        )
        .await
        .unwrap();
        assert_eq!(
            by_title,
            vec![linked_ours, linked_other, adhoc, ours, other_series],
            "keyless title fallback still matches keyed priors (pre-batch semantics)"
        );
    }

    /// specs/0041 WS4: a manually linked meeting (meeting_series_links) qualifies as a
    /// prior, and a row hit by several arms at once still comes back exactly once.
    #[tokio::test]
    async fn prior_series_occurrences_manual_link_arm_and_dedup() {
        let pool = memory_db().await;

        let keyed =
            recorded_with_summary(&pool, "Weekly", "2026-06-08T10:00:00Z", Some("series-A")).await;
        // Ad-hoc recording, different title, NULL key — only reachable via a manual link.
        let linked =
            recorded_with_summary(&pool, "Quick chat with Sam", "2026-06-10T10:00:00Z", None).await;
        sqlx::query(
            "INSERT INTO meeting_series_links (meeting_id, series_key) VALUES (?, 'series-A')",
        )
        .bind(&linked)
        .execute(&pool)
        .await
        .unwrap();
        // Triple hit: same key + same title + a redundant manual link — must appear ONCE.
        let triple =
            recorded_with_summary(&pool, "Weekly", "2026-06-12T10:00:00Z", Some("series-A")).await;
        sqlx::query(
            "INSERT INTO meeting_series_links (meeting_id, series_key) VALUES (?, 'series-A')",
        )
        .bind(&triple)
        .execute(&pool)
        .await
        .unwrap();

        let prior = MeetingsRepository::find_prior_series_occurrences(
            &pool,
            Some("series-A"),
            "Weekly",
            dt("2026-06-22T10:00:00Z"),
            10,
        )
        .await
        .unwrap();
        assert_eq!(
            prior,
            vec![triple, linked.clone(), keyed],
            "manual-link arm matches; multi-arm hits are deduped"
        );

        // HAS_CONTENT still gates the manual-link arm: a linked but content-less recording
        // never qualifies.
        sqlx::query(
            "INSERT INTO meetings (id, title, created_at, updated_at, origin) \
             VALUES ('m-linked-empty', 'Empty chat', '2026-06-11T10:00:00Z', '2026-06-11T10:00:00Z', 'recorded')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO meeting_series_links (meeting_id, series_key) VALUES ('m-linked-empty', 'series-A')",
        )
        .execute(&pool)
        .await
        .unwrap();
        // A linked SCHEDULED placeholder is excluded too (origin gate on every arm).
        let sched = MeetingsRepository::upsert_scheduled_meeting(
            &pool,
            "evt-x",
            None,
            "Weekly",
            dt("2026-06-13T10:00:00Z"),
        )
        .await
        .unwrap()
        .into_id();
        sqlx::query(
            "INSERT INTO meeting_series_links (meeting_id, series_key) VALUES (?, 'series-A')",
        )
        .bind(&sched)
        .execute(&pool)
        .await
        .unwrap();

        let prior = MeetingsRepository::find_prior_series_occurrences(
            &pool,
            Some("series-A"),
            "", // key-and-link arms only
            dt("2026-06-22T10:00:00Z"),
            10,
        )
        .await
        .unwrap();
        assert!(!prior.contains(&"m-linked-empty".to_string()));
        assert!(!prior.contains(&sched));
        assert!(prior.contains(&linked));
    }

    /// specs/0041 WS4: link/unlink semantics — key resolution order, minted `manual:` keys
    /// stamped on BOTH rows (via the links table, never meetings.calendar_series_key), and
    /// cascade on meeting delete.
    #[tokio::test]
    async fn link_meeting_to_series_resolution_and_cascade() {
        let pool = memory_db().await;

        // 1. Target with a calendar key → the prior gets a link row with THAT key.
        let target =
            recorded_with_summary(&pool, "Weekly", "2026-06-15T10:00:00Z", Some("cal-key")).await;
        let prior = recorded_with_summary(&pool, "Chat", "2026-06-10T10:00:00Z", None).await;
        let key = MeetingsRepository::link_meeting_to_series(&pool, &prior, &target)
            .await
            .unwrap();
        assert_eq!(key, "cal-key");
        assert_eq!(
            MeetingsRepository::get_series_link(&pool, &prior)
                .await
                .unwrap()
                .as_deref(),
            Some("cal-key")
        );
        // The target already carries the key on the meetings row — no link row minted for it.
        assert_eq!(
            MeetingsRepository::get_series_link(&pool, &target)
                .await
                .unwrap(),
            None
        );
        // The matcher now finds the linked prior through the effective key.
        let found = MeetingsRepository::find_prior_series_occurrences(
            &pool,
            MeetingsRepository::resolve_effective_series_key(&pool, &target)
                .await
                .unwrap()
                .as_deref(),
            "",
            dt("2026-06-15T10:00:00Z"),
            10,
        )
        .await
        .unwrap();
        assert_eq!(found, vec![prior.clone()]);

        // 2. Neither side has a key → a manual:{uuid} key is minted and stamped on BOTH via
        //    the links table; meetings.calendar_series_key stays NULL on both.
        let adhoc_target =
            recorded_with_summary(&pool, "One-off", "2026-06-16T10:00:00Z", None).await;
        let adhoc_prior =
            recorded_with_summary(&pool, "Older chat", "2026-06-09T10:00:00Z", None).await;
        let minted = MeetingsRepository::link_meeting_to_series(&pool, &adhoc_prior, &adhoc_target)
            .await
            .unwrap();
        assert!(minted.starts_with("manual:"), "minted key: {minted}");
        for id in [&adhoc_target, &adhoc_prior] {
            assert_eq!(
                MeetingsRepository::get_series_link(&pool, id)
                    .await
                    .unwrap()
                    .as_deref(),
                Some(minted.as_str())
            );
            let cal: Option<Option<String>> =
                sqlx::query_scalar("SELECT calendar_series_key FROM meetings WHERE id = ?")
                    .bind(id)
                    .fetch_optional(&pool)
                    .await
                    .unwrap();
            assert_eq!(
                cal.flatten(),
                None,
                "meetings.calendar_series_key must never be mutated by linking"
            );
        }

        // 3. Target whose key comes from ITS link row (order 2): linking another meeting to
        //    the adhoc target reuses the minted key instead of minting a second one.
        let third = recorded_with_summary(&pool, "Third", "2026-06-08T10:00:00Z", None).await;
        let reused = MeetingsRepository::link_meeting_to_series(&pool, &third, &adhoc_target)
            .await
            .unwrap();
        assert_eq!(reused, minted);

        // Self-link and unknown ids are rejected with actionable errors.
        assert!(
            MeetingsRepository::link_meeting_to_series(&pool, &target, &target)
                .await
                .is_err()
        );
        assert!(
            MeetingsRepository::link_meeting_to_series(&pool, "meeting-missing", &target)
                .await
                .is_err()
        );

        // Unlink removes only the link row (idempotent second call reports false).
        assert!(
            MeetingsRepository::unlink_meeting_from_series(&pool, &prior)
                .await
                .unwrap()
        );
        assert!(
            !MeetingsRepository::unlink_meeting_from_series(&pool, &prior)
                .await
                .unwrap()
        );

        // Cascade: deleting a linked meeting removes its link row.
        assert!(MeetingsRepository::delete_meeting(&pool, &adhoc_prior)
            .await
            .unwrap());
        assert_eq!(
            MeetingsRepository::get_series_link(&pool, &adhoc_prior)
                .await
                .unwrap(),
            None
        );

        // linked_meetings_for_series lists the remaining members, excluding the target.
        let members = MeetingsRepository::linked_meetings_for_series(&pool, &minted, &adhoc_target)
            .await
            .unwrap();
        assert_eq!(members.len(), 1);
        assert_eq!(members[0].id, third);
    }

    #[tokio::test]
    async fn prior_series_occurrences_excludes_content_less_and_non_recorded() {
        let pool = memory_db().await;
        // A recorded-but-content-less occurrence (no summary/transcript/notes) is skipped.
        sqlx::query(
            "INSERT INTO meetings (id, title, created_at, updated_at, origin, calendar_series_key) \
             VALUES ('m-empty', 'Weekly', '2026-06-01T10:00:00Z', '2026-06-01T10:00:00Z', 'recorded', 'S')",
        )
        .execute(&pool)
        .await
        .unwrap();
        // A scheduled (future prep) occurrence is skipped even with content.
        sqlx::query(
            "INSERT INTO meetings (id, title, created_at, updated_at, origin, calendar_series_key) \
             VALUES ('m-sched', 'Weekly', '2026-06-05T10:00:00Z', '2026-06-05T10:00:00Z', 'scheduled', 'S')",
        )
        .execute(&pool)
        .await
        .unwrap();
        let good = recorded_with_summary(&pool, "Weekly", "2026-06-03T10:00:00Z", Some("S")).await;

        let prior = MeetingsRepository::find_prior_series_occurrences(
            &pool,
            Some("S"),
            "Weekly",
            dt("2026-06-10T10:00:00Z"),
            5,
        )
        .await
        .unwrap();
        assert_eq!(
            prior,
            vec![good],
            "only the recorded, content-bearing occurrence qualifies"
        );
    }

    /// specs/0069 W3 Risks — a manual entry (specs/0069) titled like an existing series
    /// picks up title-matched carryover exactly like any other same-titled ad-hoc recording
    /// (the WS4 additive union). Exercises the same lookup `api_get_prep` performs: the
    /// manual row's own effective series key (nothing pins a fresh manual entry to a series)
    /// plus its title and occurrence start.
    #[tokio::test]
    async fn a_manual_entrys_title_matches_a_prior_series_occurrence() {
        let pool = memory_db().await;
        let prior = recorded_with_summary(&pool, "Weekly Sync", "2026-06-01T10:00:00Z", None).await;

        let manual_id = MeetingsRepository::create_manual_scheduled(
            &pool,
            "Weekly Sync",
            dt("2026-06-08T10:00:00Z"),
            None,
            None,
        )
        .await
        .unwrap();
        let meta = MeetingsRepository::get_meeting_metadata(&pool, &manual_id)
            .await
            .unwrap()
            .unwrap();
        let series_key = MeetingsRepository::resolve_effective_series_key(&pool, &manual_id)
            .await
            .unwrap();
        assert_eq!(
            series_key, None,
            "a fresh manual entry has no series affiliation of its own"
        );

        let found = MeetingsRepository::find_prior_series_occurrences(
            &pool,
            series_key.as_deref(),
            &meta.title,
            meta.created_at.0,
            10,
        )
        .await
        .unwrap();
        assert_eq!(found, vec![prior], "title arm alone surfaces the prior occurrence");
    }
}
