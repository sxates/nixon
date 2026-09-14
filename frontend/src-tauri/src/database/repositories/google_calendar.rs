//! `google_calendar_*` table access (specs/0032) — the connected-account row, per-calendar
//! sync bookkeeping, and the local event cache the agenda/upcoming/merge layers read.
//!
//! OAuth tokens are NOT stored here and never may be (ADR-0010): the refresh token lives in
//! the Keychain (`secrets::GCAL_REFRESH_TOKEN_ACCOUNT`), access tokens in memory. This repo
//! only holds non-secret provider state. Mirrors the sibling repos (returns `SqlxError`,
//! command layer maps to strings); shape follows `dismissed_calendar_event.rs`.
//!
//! Time columns are RFC3339 UTC strings (chrono `to_rfc3339()`), so lexicographic SQL
//! comparison equals chronological comparison — the window query relies on it (and the
//! merge/dismissal key alignment in spec 0032 requires the same format anyway).

use chrono::Utc;
use sqlx::{Error as SqlxError, SqlitePool};

/// One cached Google event, exactly the `google_calendar_events` row.
/// `id` is `gcal:<calendarId>/<instanceEventId>` (occurrence-unique; the `gcal:` prefix is
/// the routing discriminator downstream). `attendees_json` is the raw JSON array the sync
/// engine wrote (`[{name,email,isCurrentUser,responseStatus,isOrganizer}]`).
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct GoogleCalendarEventRow {
    pub id: String,
    pub calendar_id: String,
    /// Cross-source external identity (matches EventKit's
    /// `calendarItemExternalIdentifier` for the same event — merge-layer dedupe key).
    pub ical_uid: Option<String>,
    pub title: String,
    /// RFC3339 UTC (chrono `to_rfc3339()`).
    pub starts_at: String,
    /// RFC3339 UTC (chrono `to_rfc3339()`).
    pub ends_at: String,
    pub is_all_day: bool,
    pub location: Option<String>,
    pub zoom_url: Option<String>,
    pub organizer_email: Option<String>,
    /// The user's own RSVP: needsAction|declined|tentative|accepted (None when the user
    /// isn't an attendee, e.g. their own solo events).
    pub my_response: Option<String>,
    pub attendees_json: String,
    pub status: String,
    pub updated_at: String,
}

/// The single connected account (row id = 1; v1 supports one account).
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct GoogleCalendarAccountRow {
    pub email: String,
    pub connected_at: String,
    /// Best-effort enrichment capabilities (specs/0038 WS3, ADR-0010 amendment).
    /// `None` on every field = never probed (behaves exactly as calendar-only).
    /// A granted scope is NOT access, so these reflect what the org actually
    /// allowed at runtime, not what was requested at consent.
    pub can_expand_groups: Option<bool>,
    pub can_fetch_photos: Option<bool>,
    /// RFC3339 UTC of the last capability probe (re-probe only when stale).
    pub capabilities_probed_at: Option<String>,
}

/// The connected account's probed enrichment capabilities (specs/0038 WS3).
/// Every field is `Option` because a granted scope is not the same as access:
/// `None` = not yet probed; `Some(false)` = the org denied it (`403`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoogleCapabilities {
    pub can_expand_groups: Option<bool>,
    pub can_fetch_photos: Option<bool>,
    pub probed_at: Option<String>,
}

/// Per-calendar sync bookkeeping: the user's selection toggle plus the incremental
/// `syncToken` (`None` ⇒ a full (re)sync is needed).
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct GoogleCalendarSyncRow {
    pub calendar_id: String,
    pub summary: String,
    pub selected: bool,
    pub sync_token: Option<String>,
    pub last_synced_at: Option<String>,
    /// The `timeMax` horizon of the last FULL sync (RFC3339 UTC). Google bakes the
    /// window into the syncToken, so the engine full-resyncs when `now` nears this
    /// edge (specs/0032 horizon re-extension). `None` ⇒ full sync needed.
    pub window_ends_at: Option<String>,
}

pub struct GoogleCalendarRepository;

impl GoogleCalendarRepository {
    // -- Event cache --------------------------------------------------------

    /// Insert or fully replace a cached event (sync writes are last-writer-wins on the
    /// occurrence id — Google is the source of truth).
    pub async fn upsert_event(
        pool: &SqlitePool,
        event: &GoogleCalendarEventRow,
    ) -> Result<(), SqlxError> {
        sqlx::query(
            "INSERT INTO google_calendar_events (
                 id, calendar_id, ical_uid, title, starts_at, ends_at, is_all_day,
                 location, zoom_url, organizer_email, my_response, attendees_json,
                 status, updated_at
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(id) DO UPDATE SET
                 calendar_id = excluded.calendar_id,
                 ical_uid = excluded.ical_uid,
                 title = excluded.title,
                 starts_at = excluded.starts_at,
                 ends_at = excluded.ends_at,
                 is_all_day = excluded.is_all_day,
                 location = excluded.location,
                 zoom_url = excluded.zoom_url,
                 organizer_email = excluded.organizer_email,
                 my_response = excluded.my_response,
                 attendees_json = excluded.attendees_json,
                 status = excluded.status,
                 updated_at = excluded.updated_at",
        )
        .bind(&event.id)
        .bind(&event.calendar_id)
        .bind(&event.ical_uid)
        .bind(&event.title)
        .bind(&event.starts_at)
        .bind(&event.ends_at)
        .bind(event.is_all_day)
        .bind(&event.location)
        .bind(&event.zoom_url)
        .bind(&event.organizer_email)
        .bind(&event.my_response)
        .bind(&event.attendees_json)
        .bind(&event.status)
        .bind(&event.updated_at)
        .execute(pool)
        .await?;
        Ok(())
    }

    /// Cached events whose `attendees_json` carries at least one distribution-list
    /// marker (specs/0038 WS3 — the DL-expansion pass's work list). The cache is
    /// already window-bounded, so this is every in-window event needing a
    /// group-flatten attempt. A naive `LIKE` on the marker the mapper writes —
    /// exact enough here (the marker is a fixed JSON fragment).
    pub async fn events_with_distribution_lists(
        pool: &SqlitePool,
    ) -> Result<Vec<GoogleCalendarEventRow>, SqlxError> {
        sqlx::query_as::<_, GoogleCalendarEventRow>(
            "SELECT * FROM google_calendar_events
             WHERE attendees_json LIKE '%\"isDistributionList\":true%'",
        )
        .fetch_all(pool)
        .await
    }

    /// Every cached event's raw `attendees_json` (specs/0038 WS3 — the
    /// photo-fetch pass's work list). The cache is already window-bounded, so
    /// this is exactly the in-window attendee set worth a directory-photo look.
    pub async fn all_attendees_json(pool: &SqlitePool) -> Result<Vec<String>, SqlxError> {
        let rows =
            sqlx::query_scalar::<_, String>("SELECT attendees_json FROM google_calendar_events")
                .fetch_all(pool)
                .await?;
        Ok(rows)
    }

    /// Overwrite just one cached event's `attendees_json` (specs/0038 WS3 — the
    /// DL-expansion pass folds group members in without disturbing the rest of
    /// the row). Idempotent; a no-op (0 rows) if the id is gone.
    pub async fn set_event_attendees_json(
        pool: &SqlitePool,
        id: &str,
        attendees_json: &str,
    ) -> Result<(), SqlxError> {
        sqlx::query("UPDATE google_calendar_events SET attendees_json = ? WHERE id = ?")
            .bind(attendees_json)
            .bind(id)
            .execute(pool)
            .await?;
        Ok(())
    }

    /// Remove one cached occurrence (incremental sync's `status=cancelled` path).
    /// Idempotent — deleting an unknown id is a no-op.
    pub async fn delete_event(pool: &SqlitePool, id: &str) -> Result<(), SqlxError> {
        sqlx::query("DELETE FROM google_calendar_events WHERE id = ?")
            .bind(id)
            .execute(pool)
            .await?;
        Ok(())
    }

    /// One cached event by its `gcal:` id — the attendee-lookup routing path
    /// (spec 0032: `event_attendees_by_id` reads `attendees_json` from here).
    pub async fn get_event(
        pool: &SqlitePool,
        id: &str,
    ) -> Result<Option<GoogleCalendarEventRow>, SqlxError> {
        sqlx::query_as::<_, GoogleCalendarEventRow>(
            "SELECT * FROM google_calendar_events WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(pool)
        .await
    }

    /// Events starting in `[start_rfc3339, end_rfc3339)`, ordered by start — the
    /// agenda/upcoming feed. Applies the owner's display policy (spec 0032, decisions
    /// 2026-07-02): rows the user **declined** are excluded (the flag stays cached so a
    /// future "show declined" toggle is cheap), and **all-day** rows are skipped (parity
    /// with the EventKit path). Bounds must be RFC3339 UTC like the stored values.
    pub async fn events_between(
        pool: &SqlitePool,
        start_rfc3339: &str,
        end_rfc3339: &str,
    ) -> Result<Vec<GoogleCalendarEventRow>, SqlxError> {
        sqlx::query_as::<_, GoogleCalendarEventRow>(
            "SELECT * FROM google_calendar_events
             WHERE starts_at >= ? AND starts_at < ?
               AND is_all_day = 0
               AND (my_response IS NULL OR my_response <> 'declined')
             ORDER BY starts_at ASC",
        )
        .bind(start_rfc3339)
        .bind(end_rfc3339)
        .fetch_all(pool)
        .await
    }

    /// Drop every cached event of one calendar (deselect / calendar-removed path).
    pub async fn delete_events_for_calendar(
        pool: &SqlitePool,
        calendar_id: &str,
    ) -> Result<(), SqlxError> {
        sqlx::query("DELETE FROM google_calendar_events WHERE calendar_id = ?")
            .bind(calendar_id)
            .execute(pool)
            .await?;
        Ok(())
    }

    // -- Account (single row, id = 1) ----------------------------------------

    /// The connected account, or `None` when Google isn't connected.
    pub async fn get_account(
        pool: &SqlitePool,
    ) -> Result<Option<GoogleCalendarAccountRow>, SqlxError> {
        sqlx::query_as::<_, GoogleCalendarAccountRow>(
            "SELECT email, connected_at, can_expand_groups, can_fetch_photos,
                    capabilities_probed_at
             FROM google_calendar_account WHERE id = 1",
        )
        .fetch_optional(pool)
        .await
    }

    /// The connected account's probed enrichment capabilities, or `None` when no
    /// account is connected. `None` on every field of a connected account means
    /// it has not been probed yet (specs/0038 WS3).
    pub async fn get_capabilities(
        pool: &SqlitePool,
    ) -> Result<Option<GoogleCapabilities>, SqlxError> {
        Ok(Self::get_account(pool).await?.map(|a| GoogleCapabilities {
            can_expand_groups: a.can_expand_groups,
            can_fetch_photos: a.can_fetch_photos,
            probed_at: a.capabilities_probed_at,
        }))
    }

    /// Persist a capability probe result on the single account row (specs/0038
    /// WS3). Each flag is independent: a `None` argument leaves that column
    /// untouched (a granted scope is NOT access, and a transient/inconclusive
    /// probe must never latch a column to a value — finding #3/#4). Only a
    /// conclusive result (`Some`) is written, and `capabilities_probed_at` is
    /// stamped only when at least one column was persisted, so a fully
    /// inconclusive probe leaves the account "unprobed" for a later retry.
    /// A no-op (0 rows) when no account is connected — never an error.
    pub async fn set_capabilities(
        pool: &SqlitePool,
        can_expand_groups: Option<bool>,
        can_fetch_photos: Option<bool>,
        probed_at: &str,
    ) -> Result<(), SqlxError> {
        // COALESCE keeps the existing column value when the bound argument is
        // NULL, so each flag advances independently and no inconclusive probe
        // overwrites a previously-decided value. `capabilities_probed_at` is
        // only touched when at least one flag is conclusive (guarded by the
        // caller), so the staleness gate reflects real knowledge.
        if can_expand_groups.is_none() && can_fetch_photos.is_none() {
            return Ok(());
        }
        sqlx::query(
            "UPDATE google_calendar_account
             SET can_expand_groups = COALESCE(?, can_expand_groups),
                 can_fetch_photos = COALESCE(?, can_fetch_photos),
                 capabilities_probed_at = ?
             WHERE id = 1",
        )
        .bind(can_expand_groups)
        .bind(can_fetch_photos)
        .bind(probed_at)
        .execute(pool)
        .await?;
        Ok(())
    }

    /// Record the connected account (connect path). Reconnecting overwrites the single
    /// row and refreshes `connected_at`. When the email CHANGES (a different
    /// account is connected), the probed enrichment capabilities are reset to
    /// `NULL` so the new account re-probes fresh rather than inheriting the
    /// previous account's org policy (specs/0038 WS3, finding #10). A reconnect
    /// with the SAME address preserves the cached capabilities.
    pub async fn set_account(pool: &SqlitePool, email: &str) -> Result<(), SqlxError> {
        sqlx::query(
            "INSERT INTO google_calendar_account (id, email, connected_at) VALUES (1, ?, ?)
             ON CONFLICT(id) DO UPDATE SET
                 email = excluded.email,
                 connected_at = excluded.connected_at,
                 can_expand_groups = CASE WHEN google_calendar_account.email <> excluded.email
                                          THEN NULL ELSE google_calendar_account.can_expand_groups END,
                 can_fetch_photos = CASE WHEN google_calendar_account.email <> excluded.email
                                         THEN NULL ELSE google_calendar_account.can_fetch_photos END,
                 capabilities_probed_at = CASE WHEN google_calendar_account.email <> excluded.email
                                               THEN NULL ELSE google_calendar_account.capabilities_probed_at END",
        )
        .bind(email)
        .bind(Utc::now().to_rfc3339())
        .execute(pool)
        .await?;
        Ok(())
    }

    // -- Per-calendar sync state ---------------------------------------------

    /// All known calendars on the account, selected-first then by name (Settings order).
    pub async fn list_sync_states(
        pool: &SqlitePool,
    ) -> Result<Vec<GoogleCalendarSyncRow>, SqlxError> {
        sqlx::query_as::<_, GoogleCalendarSyncRow>(
            "SELECT calendar_id, summary, selected, sync_token, last_synced_at, window_ends_at
             FROM google_calendar_sync ORDER BY selected DESC, summary ASC",
        )
        .fetch_all(pool)
        .await
    }

    /// One calendar's sync state.
    pub async fn get_sync_state(
        pool: &SqlitePool,
        calendar_id: &str,
    ) -> Result<Option<GoogleCalendarSyncRow>, SqlxError> {
        sqlx::query_as::<_, GoogleCalendarSyncRow>(
            "SELECT calendar_id, summary, selected, sync_token, last_synced_at, window_ends_at
             FROM google_calendar_sync WHERE calendar_id = ?",
        )
        .bind(calendar_id)
        .fetch_optional(pool)
        .await
    }

    /// Register a calendar from a calendarList fetch. `default_selected` applies to
    /// NEW rows only (specs/0041 WS5 — a large account defaults to primary-only);
    /// on re-fetch only the display name is refreshed — the user's selection and the
    /// incremental `sync_token` are preserved (a rename must not force a full resync).
    pub async fn upsert_sync_state(
        pool: &SqlitePool,
        calendar_id: &str,
        summary: &str,
        default_selected: bool,
    ) -> Result<(), SqlxError> {
        sqlx::query(
            "INSERT INTO google_calendar_sync (calendar_id, summary, selected) VALUES (?, ?, ?)
             ON CONFLICT(calendar_id) DO UPDATE SET summary = excluded.summary",
        )
        .bind(calendar_id)
        .bind(summary)
        .bind(default_selected)
        .execute(pool)
        .await?;
        Ok(())
    }

    /// Toggle a calendar's per-user sync selection.
    pub async fn set_calendar_selected(
        pool: &SqlitePool,
        calendar_id: &str,
        selected: bool,
    ) -> Result<(), SqlxError> {
        sqlx::query("UPDATE google_calendar_sync SET selected = ? WHERE calendar_id = ?")
            .bind(selected)
            .bind(calendar_id)
            .execute(pool)
            .await?;
        Ok(())
    }

    /// Record a sync pass: the new `syncToken` (or `None` to force a full resync, e.g.
    /// after HTTP 410 GONE) and a fresh `last_synced_at`.
    pub async fn set_sync_progress(
        pool: &SqlitePool,
        calendar_id: &str,
        sync_token: Option<&str>,
    ) -> Result<(), SqlxError> {
        sqlx::query(
            "UPDATE google_calendar_sync SET sync_token = ?, last_synced_at = ?
             WHERE calendar_id = ?",
        )
        .bind(sync_token)
        .bind(Utc::now().to_rfc3339())
        .bind(calendar_id)
        .execute(pool)
        .await?;
        Ok(())
    }

    /// Record the `timeMax` horizon a FULL sync just used (specs/0032 horizon
    /// re-extension — see [`GoogleCalendarSyncRow::window_ends_at`]).
    pub async fn set_window_horizon(
        pool: &SqlitePool,
        calendar_id: &str,
        window_ends_at: &str,
    ) -> Result<(), SqlxError> {
        sqlx::query("UPDATE google_calendar_sync SET window_ends_at = ? WHERE calendar_id = ?")
            .bind(window_ends_at)
            .bind(calendar_id)
            .execute(pool)
            .await?;
        Ok(())
    }

    // -- Disconnect -----------------------------------------------------------

    /// Clear all Google Calendar state (disconnect path): account, sync bookkeeping, and
    /// the event cache — atomically. `dismissed_calendar_events` is deliberately NOT
    /// touched: dismissal keys are cross-source (0029 WS6.2) and must survive disconnect.
    pub async fn purge_all(pool: &SqlitePool) -> Result<(), SqlxError> {
        let mut tx = pool.begin().await?;
        sqlx::query("DELETE FROM google_calendar_events")
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM google_calendar_sync")
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM google_calendar_account")
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    /// Fresh in-memory SQLite brought up through the app's real migration set (mirrors
    /// repositories/meeting.rs tests) — so these tests also prove the 0032 migration runs.
    /// One connection max: each in-memory connection is a separate database.
    async fn memory_db() -> SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory sqlite");
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .expect("migrations");
        pool
    }

    fn event(id: &str, starts_at: &str) -> GoogleCalendarEventRow {
        GoogleCalendarEventRow {
            id: format!("gcal:primary/{id}"),
            calendar_id: "primary".into(),
            ical_uid: Some(format!("{id}@google.com")),
            title: format!("Event {id}"),
            starts_at: starts_at.into(),
            ends_at: "2026-07-02T11:00:00+00:00".into(),
            is_all_day: false,
            location: None,
            zoom_url: None,
            organizer_email: Some("organizer@example.com".into()),
            my_response: Some("accepted".into()),
            attendees_json: "[]".into(),
            status: "confirmed".into(),
            updated_at: "2026-07-01T00:00:00+00:00".into(),
        }
    }

    #[tokio::test]
    async fn upsert_then_window_query_returns_events_in_start_order() {
        let pool = memory_db().await;
        let late = event("late", "2026-07-02T15:00:00+00:00");
        let early = event("early", "2026-07-02T09:00:00+00:00");
        let outside = event("tomorrow", "2026-07-03T09:00:00+00:00");
        for e in [&late, &early, &outside] {
            GoogleCalendarRepository::upsert_event(&pool, e)
                .await
                .unwrap();
        }

        let got = GoogleCalendarRepository::events_between(
            &pool,
            "2026-07-02T00:00:00+00:00",
            "2026-07-03T00:00:00+00:00",
        )
        .await
        .unwrap();
        assert_eq!(
            got.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(),
            vec!["gcal:primary/early", "gcal:primary/late"],
            "in-window events only, ordered by starts_at"
        );
        assert_eq!(got[0], early, "the full row round-trips");
    }

    #[tokio::test]
    async fn upsert_is_last_writer_wins_on_the_occurrence_id() {
        let pool = memory_db().await;
        let mut e = event("a", "2026-07-02T09:00:00+00:00");
        GoogleCalendarRepository::upsert_event(&pool, &e)
            .await
            .unwrap();
        e.title = "Renamed".into();
        e.my_response = Some("tentative".into());
        GoogleCalendarRepository::upsert_event(&pool, &e)
            .await
            .unwrap();

        let got = GoogleCalendarRepository::get_event(&pool, &e.id)
            .await
            .unwrap()
            .expect("row exists");
        assert_eq!(got.title, "Renamed");
        assert_eq!(got.my_response.as_deref(), Some("tentative"));
    }

    #[tokio::test]
    async fn window_query_excludes_declined_events() {
        let pool = memory_db().await;
        let mut declined = event("declined", "2026-07-02T09:00:00+00:00");
        declined.my_response = Some("declined".into());
        // NULL my_response (own solo event) must still be included.
        let mut solo = event("solo", "2026-07-02T10:00:00+00:00");
        solo.my_response = None;
        for e in [&declined, &solo] {
            GoogleCalendarRepository::upsert_event(&pool, e)
                .await
                .unwrap();
        }

        let got = GoogleCalendarRepository::events_between(
            &pool,
            "2026-07-02T00:00:00+00:00",
            "2026-07-03T00:00:00+00:00",
        )
        .await
        .unwrap();
        assert_eq!(
            got.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(),
            vec!["gcal:primary/solo"],
            "declined hidden by default; the row itself stays cached"
        );
        // The declined row is still in the cache (future "show declined" toggle).
        assert!(GoogleCalendarRepository::get_event(&pool, &declined.id)
            .await
            .unwrap()
            .is_some());
    }

    #[tokio::test]
    async fn window_query_excludes_all_day_events() {
        let pool = memory_db().await;
        let mut all_day = event("allday", "2026-07-02T00:00:00+00:00");
        all_day.is_all_day = true;
        let timed = event("timed", "2026-07-02T09:00:00+00:00");
        for e in [&all_day, &timed] {
            GoogleCalendarRepository::upsert_event(&pool, e)
                .await
                .unwrap();
        }

        let got = GoogleCalendarRepository::events_between(
            &pool,
            "2026-07-02T00:00:00+00:00",
            "2026-07-03T00:00:00+00:00",
        )
        .await
        .unwrap();
        assert_eq!(
            got.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(),
            vec!["gcal:primary/timed"],
            "all-day skipped (parity with the EventKit path)"
        );
    }

    #[tokio::test]
    async fn cancelled_instance_delete_is_idempotent() {
        let pool = memory_db().await;
        let e = event("gone", "2026-07-02T09:00:00+00:00");
        GoogleCalendarRepository::upsert_event(&pool, &e)
            .await
            .unwrap();
        GoogleCalendarRepository::delete_event(&pool, &e.id)
            .await
            .unwrap();
        assert!(GoogleCalendarRepository::get_event(&pool, &e.id)
            .await
            .unwrap()
            .is_none());
        // Deleting again (or an id we never had) is a clean no-op.
        GoogleCalendarRepository::delete_event(&pool, &e.id)
            .await
            .unwrap();
        GoogleCalendarRepository::delete_event(&pool, "gcal:primary/never-existed")
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn account_single_row_get_set_roundtrip() {
        let pool = memory_db().await;
        assert!(GoogleCalendarRepository::get_account(&pool)
            .await
            .unwrap()
            .is_none());

        GoogleCalendarRepository::set_account(&pool, "user@example.com")
            .await
            .unwrap();
        let account = GoogleCalendarRepository::get_account(&pool)
            .await
            .unwrap()
            .expect("connected");
        assert_eq!(account.email, "user@example.com");

        // Reconnect overwrites the single row (id = 1) instead of adding another.
        GoogleCalendarRepository::set_account(&pool, "other@example.com")
            .await
            .unwrap();
        let account = GoogleCalendarRepository::get_account(&pool)
            .await
            .unwrap()
            .expect("still connected");
        assert_eq!(account.email, "other@example.com");
    }

    #[tokio::test]
    async fn set_capabilities_updates_each_flag_independently() {
        let pool = memory_db().await;
        GoogleCalendarRepository::set_account(&pool, "user@example.com")
            .await
            .unwrap();

        // A conclusive group result with an INCONCLUSIVE (None) photos result
        // stores only the group flag and never latches photos to false (#3/#4).
        GoogleCalendarRepository::set_capabilities(
            &pool,
            Some(true),
            None,
            "2026-07-07T00:00:00+00:00",
        )
        .await
        .unwrap();
        let caps = GoogleCalendarRepository::get_capabilities(&pool)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(caps.can_expand_groups, Some(true));
        assert_eq!(
            caps.can_fetch_photos, None,
            "None photos must stay unprobed"
        );
        assert_eq!(caps.probed_at.as_deref(), Some("2026-07-07T00:00:00+00:00"));

        // A later conclusive photos denial (Some(false)) stores without
        // disturbing the previously-decided group flag.
        GoogleCalendarRepository::set_capabilities(
            &pool,
            None,
            Some(false),
            "2026-07-08T00:00:00+00:00",
        )
        .await
        .unwrap();
        let caps = GoogleCalendarRepository::get_capabilities(&pool)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(caps.can_expand_groups, Some(true), "group flag preserved");
        assert_eq!(caps.can_fetch_photos, Some(false), "photos denial stored");
        assert_eq!(caps.probed_at.as_deref(), Some("2026-07-08T00:00:00+00:00"));

        // A fully inconclusive probe (both None) is a no-op — it must not stamp
        // probed_at, so the account stays due for a retry.
        GoogleCalendarRepository::set_capabilities(&pool, None, None, "2026-07-09T00:00:00+00:00")
            .await
            .unwrap();
        let caps = GoogleCalendarRepository::get_capabilities(&pool)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            caps.probed_at.as_deref(),
            Some("2026-07-08T00:00:00+00:00"),
            "a fully inconclusive probe leaves probed_at unchanged"
        );
    }

    #[tokio::test]
    async fn set_account_resets_capabilities_only_when_email_changes() {
        let pool = memory_db().await;
        GoogleCalendarRepository::set_account(&pool, "user@example.com")
            .await
            .unwrap();
        GoogleCalendarRepository::set_capabilities(
            &pool,
            Some(true),
            Some(true),
            "2026-07-07T00:00:00+00:00",
        )
        .await
        .unwrap();

        // Reconnecting the SAME account preserves the probed capabilities.
        GoogleCalendarRepository::set_account(&pool, "user@example.com")
            .await
            .unwrap();
        let caps = GoogleCalendarRepository::get_capabilities(&pool)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(caps.can_expand_groups, Some(true));
        assert_eq!(caps.can_fetch_photos, Some(true));
        assert!(caps.probed_at.is_some());

        // Connecting a DIFFERENT account resets them to NULL so it re-probes
        // fresh rather than inheriting the previous org's policy (finding #10).
        GoogleCalendarRepository::set_account(&pool, "other@example.com")
            .await
            .unwrap();
        let caps = GoogleCalendarRepository::get_capabilities(&pool)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(caps.can_expand_groups, None);
        assert_eq!(caps.can_fetch_photos, None);
        assert_eq!(caps.probed_at, None);
    }

    #[tokio::test]
    async fn sync_state_upsert_preserves_selection_and_token_on_summary_refresh() {
        let pool = memory_db().await;
        GoogleCalendarRepository::upsert_sync_state(&pool, "cal-1", "Work", true)
            .await
            .unwrap();
        let state = GoogleCalendarRepository::get_sync_state(&pool, "cal-1")
            .await
            .unwrap()
            .expect("registered");
        assert!(state.selected, "new calendars default to selected");
        assert_eq!(state.sync_token, None, "no token yet => full sync needed");

        // User deselects; a sync pass stores a token.
        GoogleCalendarRepository::set_calendar_selected(&pool, "cal-1", false)
            .await
            .unwrap();
        GoogleCalendarRepository::set_sync_progress(&pool, "cal-1", Some("tok-1"))
            .await
            .unwrap();

        // A calendarList re-fetch (rename) must not clobber either.
        GoogleCalendarRepository::upsert_sync_state(&pool, "cal-1", "Work (renamed)", true)
            .await
            .unwrap();
        let state = GoogleCalendarRepository::get_sync_state(&pool, "cal-1")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(state.summary, "Work (renamed)");
        assert!(!state.selected);
        assert_eq!(state.sync_token.as_deref(), Some("tok-1"));
        assert!(state.last_synced_at.is_some());

        // Clearing the token (410 GONE) forces the next pass to full-resync.
        GoogleCalendarRepository::set_sync_progress(&pool, "cal-1", None)
            .await
            .unwrap();
        let state = GoogleCalendarRepository::get_sync_state(&pool, "cal-1")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(state.sync_token, None);
    }

    #[tokio::test]
    async fn upsert_sync_state_default_selected_applies_to_new_rows_only() {
        let pool = memory_db().await;

        // A NEW row honors the explicit default (specs/0041 WS5: large accounts
        // register non-primary calendars unselected).
        GoogleCalendarRepository::upsert_sync_state(&pool, "cal-off", "Off by default", false)
            .await
            .unwrap();
        let state = GoogleCalendarRepository::get_sync_state(&pool, "cal-off")
            .await
            .unwrap()
            .expect("registered");
        assert!(!state.selected, "new row starts with the explicit default");

        // The user turns it on; a later re-registration with default=false must
        // NOT clobber the user's choice (only the name refreshes).
        GoogleCalendarRepository::set_calendar_selected(&pool, "cal-off", true)
            .await
            .unwrap();
        GoogleCalendarRepository::upsert_sync_state(&pool, "cal-off", "Renamed", false)
            .await
            .unwrap();
        let state = GoogleCalendarRepository::get_sync_state(&pool, "cal-off")
            .await
            .unwrap()
            .unwrap();
        assert!(state.selected, "existing selection survives the upsert");
        assert_eq!(state.summary, "Renamed");

        // Symmetric: a user-deselected row survives an upsert with default=true.
        GoogleCalendarRepository::upsert_sync_state(&pool, "cal-on", "On by default", true)
            .await
            .unwrap();
        GoogleCalendarRepository::set_calendar_selected(&pool, "cal-on", false)
            .await
            .unwrap();
        GoogleCalendarRepository::upsert_sync_state(&pool, "cal-on", "On by default", true)
            .await
            .unwrap();
        let state = GoogleCalendarRepository::get_sync_state(&pool, "cal-on")
            .await
            .unwrap()
            .unwrap();
        assert!(!state.selected, "user's OFF survives a default-ON upsert");
    }

    #[tokio::test]
    async fn list_sync_states_orders_selected_first_then_by_name() {
        let pool = memory_db().await;
        for (id, name) in [("cal-b", "Beta"), ("cal-a", "Alpha"), ("cal-c", "Chores")] {
            GoogleCalendarRepository::upsert_sync_state(&pool, id, name, true)
                .await
                .unwrap();
        }
        GoogleCalendarRepository::set_calendar_selected(&pool, "cal-a", false)
            .await
            .unwrap();

        let states = GoogleCalendarRepository::list_sync_states(&pool)
            .await
            .unwrap();
        assert_eq!(
            states
                .iter()
                .map(|s| s.calendar_id.as_str())
                .collect::<Vec<_>>(),
            vec!["cal-b", "cal-c", "cal-a"]
        );
    }

    #[tokio::test]
    async fn delete_events_for_calendar_only_touches_that_calendar() {
        let pool = memory_db().await;
        let mut other = event("other", "2026-07-02T09:00:00+00:00");
        other.calendar_id = "cal-2".into();
        let mine = event("mine", "2026-07-02T10:00:00+00:00");
        for e in [&other, &mine] {
            GoogleCalendarRepository::upsert_event(&pool, e)
                .await
                .unwrap();
        }

        GoogleCalendarRepository::delete_events_for_calendar(&pool, "cal-2")
            .await
            .unwrap();
        assert!(GoogleCalendarRepository::get_event(&pool, &other.id)
            .await
            .unwrap()
            .is_none());
        assert!(GoogleCalendarRepository::get_event(&pool, &mine.id)
            .await
            .unwrap()
            .is_some());
    }

    #[tokio::test]
    async fn purge_all_clears_every_google_table_but_not_dismissals() {
        let pool = memory_db().await;
        GoogleCalendarRepository::set_account(&pool, "user@example.com")
            .await
            .unwrap();
        GoogleCalendarRepository::upsert_sync_state(&pool, "cal-1", "Work", true)
            .await
            .unwrap();
        GoogleCalendarRepository::upsert_event(&pool, &event("e1", "2026-07-02T09:00:00+00:00"))
            .await
            .unwrap();
        // Dismissal keys are cross-source and must survive a disconnect (0029 WS6.2).
        crate::database::repositories::dismissed_calendar_event::DismissedCalendarEventsRepository
            ::dismiss(&pool, "ext:e1@google.com@2026-07-02T09:00:00+00:00")
            .await
            .unwrap();

        GoogleCalendarRepository::purge_all(&pool).await.unwrap();

        assert!(GoogleCalendarRepository::get_account(&pool)
            .await
            .unwrap()
            .is_none());
        assert!(GoogleCalendarRepository::list_sync_states(&pool)
            .await
            .unwrap()
            .is_empty());
        let events = GoogleCalendarRepository::events_between(
            &pool,
            "2026-01-01T00:00:00+00:00",
            "2027-01-01T00:00:00+00:00",
        )
        .await
        .unwrap();
        assert!(events.is_empty());
        let dismissed =
            crate::database::repositories::dismissed_calendar_event::DismissedCalendarEventsRepository::all(&pool)
                .await
                .unwrap();
        assert_eq!(dismissed.len(), 1, "dismissals are retained");
    }
}
