//! Fold an auto-created person into the singleton owner ("You") — the heart of the
//! "This is me" claim (specs/0018 Phase A Task 2).
//!
//! When EventKit's `isCurrentUser()` misses you (you were invited under an alias /
//! personal / delegated address), `seed_from_attendees` auto-creates an ordinary
//! `people` row for that address and rosters it. Claiming it as "me" must collapse that
//! row into the existing owner singleton ([`OWNER_PERSON_ID`]) — re-pointing every
//! person-keyed row onto the owner and deleting the now-empty person — so the app keeps a
//! SINGLE owner identity (the invariant specs/0016 established).
//!
//! All cascades are EXPLICIT in one transaction because the app pool connects without
//! `PRAGMA foreign_keys = ON` (mirrors [`PeopleRepository::delete`]). The
//! person-referencing tables today are `speakers.person_id`, `voiceprints.person_id`,
//! `meeting_participants.person_id`, and `action_items.assignee_person_id` (specs/0034);
//! each is handled below — add a step here whenever a new table gains a person FK.
//!
//! [`PeopleRepository::delete`]: crate::database::repositories::people::PeopleRepository::delete

use anyhow::{Context, Result};
use sqlx::SqlitePool;

use crate::people::enroll::{ensure_owner_person, OWNER_PERSON_ID};

/// Fold an auto-created person into the singleton owner ([`OWNER_PERSON_ID`]).
///
/// One transaction, explicit cascade (no `PRAGMA foreign_keys`):
/// 1. Ensure the owner person exists (idempotent target).
/// 2. Re-point `speakers.person_id` → owner (keep the per-meeting speaker rows; only the
///    identity link moves).
/// 3. Re-point `voiceprints.person_id` → owner (decision V — it IS the owner's voice; we
///    re-point, never delete — no new enrollment occurs).
/// 4. Delete the folded person's `meeting_participants` rows (the owner is not a
///    participant in any meeting; re-pointing could also violate the
///    `(meeting_id, person_id)` PK).
/// 5. Re-point `action_items` assignments (specs/0034): the folded person's tasks ARE
///    the owner's tasks — set `assignee_is_self = 1` and NULL the FK (the owner is not a
///    `people` row, so "assigned to me" is the flag, mirroring extraction's resolution).
/// 6. Delete the now-empty `people` row.
///
/// Idempotent and a NO-OP when `person_id == OWNER_PERSON_ID`. Returns `Ok(())` even when
/// the person is absent (steps 2–5 affect 0 rows). Safe to call twice with the same id.
pub async fn merge_person_into_owner(pool: &SqlitePool, person_id: &str) -> Result<()> {
    // Claiming the owner as the owner is a no-op (and re-pointing owner→owner then
    // deleting the owner row would be a bug — guard before any write).
    if person_id == OWNER_PERSON_ID {
        return Ok(());
    }

    // Guarantee the merge target exists before re-pointing anything onto it.
    ensure_owner_person(pool)
        .await
        .context("ensure owner person before merge")?;

    let mut tx = pool.begin().await.context("begin merge transaction")?;

    // 2. Speakers: move the identity link to the owner (keep display_name/email as-is —
    //    the transcript label is a separate concern from "this is me").
    sqlx::query("UPDATE speakers SET person_id = ? WHERE person_id = ?")
        .bind(OWNER_PERSON_ID)
        .bind(person_id)
        .execute(&mut *tx)
        .await
        .context("re-point speakers to owner")?;

    // 3. Voiceprints (decision V — re-point, NOT delete): it is the owner's voice. No new
    //    enrollment; future enrollment stays gated by store_voiceprints (specs/0078).
    sqlx::query("UPDATE voiceprints SET person_id = ? WHERE person_id = ?")
        .bind(OWNER_PERSON_ID)
        .bind(person_id)
        .execute(&mut *tx)
        .await
        .context("re-point voiceprints to owner")?;

    // 4. Roster: the owner is never a participant — drop the folded person's join rows
    //    app-wide rather than re-point (avoids the (meeting_id, person_id) PK collision and
    //    keeps the owner off every roster).
    sqlx::query("DELETE FROM meeting_participants WHERE person_id = ?")
        .bind(person_id)
        .execute(&mut *tx)
        .await
        .context("remove folded person's participant rows")?;

    // 5. Action items (specs/0034): tasks assigned to the folded person are the OWNER's
    //    tasks — flip to the self flag and clear the FK (the owner is deliberately not a
    //    `people` row), matching how speakers/voiceprints re-point rather than strand.
    sqlx::query(
        "UPDATE action_items SET assignee_is_self = 1, assignee_person_id = NULL
         WHERE assignee_person_id = ?",
    )
    .bind(person_id)
    .execute(&mut *tx)
    .await
    .context("re-point action-item assignments to owner")?;

    // 6. Delete the now-empty person (guarded != owner above, so we never delete "You").
    sqlx::query("DELETE FROM people WHERE id = ?")
        .bind(person_id)
        .execute(&mut *tx)
        .await
        .context("delete folded person row")?;

    tx.commit().await.context("commit merge transaction")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    /// In-memory pool brought up through the app's REAL migration set, so the merge's
    /// explicit cascade always runs against the tables as they actually exist (a
    /// hand-rolled schema here silently drifted when specs/0034 added
    /// `action_items.assignee_person_id`). One connection max — each in-memory
    /// connection is a separate database. `foreign_keys` is ON by sqlx default, matching
    /// the app pool (manager.rs) — hence the real meeting row below for speakers' FK.
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

    /// Real meeting row (speakers declares `meeting_id REFERENCES meetings(id)` and the
    /// pool enforces it). Returns the meeting id.
    async fn create_meeting(pool: &SqlitePool) -> String {
        crate::database::repositories::meeting::MeetingsRepository::create_meeting(
            pool,
            Some("Sync".into()),
            None,
            None,
            None,
            None,
        )
        .await
        .expect("create_meeting")
    }

    /// Insert a person row directly (bypassing upsert-by-email) so tests can place rows
    /// with arbitrary ids.
    async fn insert_person(pool: &SqlitePool, id: &str, email: Option<&str>) {
        sqlx::query(
            "INSERT INTO people (id, email, display_name, voiceprint_opt_out, created_at, updated_at)
             VALUES (?, ?, 'X', 0, '', '')",
        )
        .bind(id)
        .bind(email)
        .execute(pool)
        .await
        .unwrap();
    }

    async fn count(pool: &SqlitePool, sql: &str, id: &str) -> i64 {
        sqlx::query_scalar::<_, i64>(sql)
            .bind(id)
            .fetch_one(pool)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn merge_repoints_speakers_voiceprints_and_drops_roster_and_person() {
        let pool = test_pool().await;
        let meeting_id = create_meeting(&pool).await;
        insert_person(&pool, "p1", Some("me@work.com")).await;

        // A speaker, a voiceprint, and a roster row all keyed to p1.
        sqlx::query(
            "INSERT INTO speakers (id, meeting_id, speaker_key, display_name, person_id, created_at, updated_at)
             VALUES ('s1', ?,'spk1','Speaker 1','p1','','')",
        )
        .bind(&meeting_id)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO voiceprints (id, person_id, embedding, embedding_dim, embedding_model, created_at)
             VALUES ('vp1','p1', X'00', 1, 'model', '')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO meeting_participants (meeting_id, person_id, source, created_at)
             VALUES (?,'p1','calendar','')",
        )
        .bind(&meeting_id)
        .execute(&pool)
        .await
        .unwrap();

        merge_person_into_owner(&pool, "p1").await.unwrap();

        // speakers + voiceprints re-pointed to the owner.
        assert_eq!(
            count(
                &pool,
                "SELECT COUNT(*) FROM speakers WHERE person_id = ?",
                OWNER_PERSON_ID
            )
            .await,
            1,
            "speaker re-pointed to owner"
        );
        assert_eq!(
            count(
                &pool,
                "SELECT COUNT(*) FROM voiceprints WHERE person_id = ?",
                OWNER_PERSON_ID
            )
            .await,
            1,
            "voiceprint re-pointed to owner (NOT deleted)"
        );
        // roster row gone, p1 deleted, owner survives.
        assert_eq!(
            count(
                &pool,
                "SELECT COUNT(*) FROM meeting_participants WHERE person_id = ?",
                "p1"
            )
            .await,
            0,
            "folded person's roster rows removed"
        );
        assert_eq!(
            count(&pool, "SELECT COUNT(*) FROM people WHERE id = ?", "p1").await,
            0
        );
        assert_eq!(
            count(
                &pool,
                "SELECT COUNT(*) FROM people WHERE id = ?",
                OWNER_PERSON_ID
            )
            .await,
            1,
            "owner survives"
        );
    }

    /// Finding (specs/0034): "claim as me" must not strand task assignments — items
    /// assigned to the folded person become the OWNER's items (`assignee_is_self = 1`,
    /// FK cleared), mirroring the speaker/voiceprint re-point.
    #[tokio::test]
    async fn merge_repoints_action_item_assignments_to_self() {
        let pool = test_pool().await;
        insert_person(&pool, "p1", Some("me@work.com")).await;
        insert_person(&pool, "p2", Some("other@x.com")).await;
        sqlx::query(
            "INSERT INTO action_items (id, meeting_id, description, assignee_person_id, content_key, created_at, updated_at)
             VALUES ('ai-1','m1','send the deck','p1','k1','',''),
                    ('ai-2','m1','book the room','p2','k2','','')",
        )
        .execute(&pool)
        .await
        .unwrap();

        merge_person_into_owner(&pool, "p1").await.unwrap();

        let (is_self, person_id): (bool, Option<String>) = sqlx::query_as(
            "SELECT assignee_is_self, assignee_person_id FROM action_items WHERE id = 'ai-1'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(is_self, "folded person's task re-pointed to the owner flag");
        assert!(person_id.is_none(), "stale person FK cleared");

        // Another person's assignment is untouched.
        let (other_self, other_person): (bool, Option<String>) = sqlx::query_as(
            "SELECT assignee_is_self, assignee_person_id FROM action_items WHERE id = 'ai-2'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(!other_self);
        assert_eq!(other_person.as_deref(), Some("p2"));
    }

    #[tokio::test]
    async fn merge_is_idempotent() {
        let pool = test_pool().await;
        let meeting_id = create_meeting(&pool).await;
        insert_person(&pool, "p1", None).await;
        sqlx::query(
            "INSERT INTO speakers (id, meeting_id, speaker_key, display_name, person_id, created_at, updated_at)
             VALUES ('s1', ?,'spk1','Speaker 1','p1','','')",
        )
        .bind(&meeting_id)
        .execute(&pool)
        .await
        .unwrap();

        merge_person_into_owner(&pool, "p1").await.unwrap();
        // Second call with the now-deleted id is a safe no-op.
        merge_person_into_owner(&pool, "p1").await.unwrap();

        assert_eq!(
            count(
                &pool,
                "SELECT COUNT(*) FROM speakers WHERE person_id = ?",
                OWNER_PERSON_ID
            )
            .await,
            1
        );
        assert_eq!(
            count(&pool, "SELECT COUNT(*) FROM people WHERE id = ?", "p1").await,
            0
        );
    }

    #[tokio::test]
    async fn merge_is_noop_for_owner_id() {
        let pool = test_pool().await;
        ensure_owner_person(&pool).await.unwrap();
        sqlx::query(
            "INSERT INTO voiceprints (id, person_id, embedding, embedding_dim, embedding_model, created_at)
             VALUES ('vp1', ?, X'00', 1, 'model', '')",
        )
        .bind(OWNER_PERSON_ID)
        .execute(&pool)
        .await
        .unwrap();

        merge_person_into_owner(&pool, OWNER_PERSON_ID)
            .await
            .unwrap();

        // Owner row and its voiceprint untouched.
        assert_eq!(
            count(
                &pool,
                "SELECT COUNT(*) FROM people WHERE id = ?",
                OWNER_PERSON_ID
            )
            .await,
            1
        );
        assert_eq!(
            count(
                &pool,
                "SELECT COUNT(*) FROM voiceprints WHERE person_id = ?",
                OWNER_PERSON_ID
            )
            .await,
            1
        );
    }
}
