//! Google Calendar sync engine (specs/0032 task 3, ADR-0010) — a poll-based
//! incremental sync of the user's selected calendars into the local
//! `google_calendar_events` SQLite cache. Read-only, metadata-only; the agenda
//! and attendee lookups are always served from the cache, never per-request
//! API calls.
//!
//! Sync model (spec 0032 "Sync model"):
//!   - **Full sync** per selected calendar: `events.list` with
//!     `singleEvents=true` over a `[now-14d, now+60d)` window, paged via
//!     `nextPageToken`; the final page's `nextSyncToken` is stored for
//!     incremental passes. A full sync REPLACES the calendar's cached rows,
//!     which is also the window prune (rows outside the re-extended window die
//!     with the replace).
//!   - **Incremental sync**: same endpoint with `syncToken` only.
//!     `status=cancelled` instances delete their cache row; HTTP `410 GONE`
//!     clears the token and transparently falls back to a full resync.
//!   - **Horizon re-extension**: Google bakes the window into the syncToken,
//!     so each full sync persists its `timeMax` horizon
//!     (`google_calendar_sync.window_ends_at`); once `now` is within 7 days of
//!     that edge the next pass full-resyncs to re-extend the window.
//!   - **Triggers**: connect, app-window focus, agenda/upcoming builds via
//!     [`sync_if_stale`] (5-minute staleness), a 10-minute background timer
//!     ([`spawn_background_sync`], owner decision 2026-07-02), and manual
//!     "Sync now". A single-flight guard coalesces concurrent triggers.
//!   - **Errors**: offline/API failures are logged and the cache keeps
//!     serving (the agenda is already best-effort); `403/429` quota errors are
//!     simply skipped until the next trigger (no retry storm — the trigger
//!     cadence IS the backoff for v1); a refresh rejected with `invalid_grant`
//!     emits [`AUTH_REQUIRED_EVENT`] once and latches [`sync_all`] off until
//!     the user reconnects (or disconnects), so we never error-loop.
//!
//! The HTTP layer is injected into the per-calendar engine as a plain
//! `Fn(EventsRequest) -> Future` so the unit tests below run over canned
//! `events.list` JSON fixtures with **no network**.

use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{anyhow, bail, Context, Result};
use chrono::{DateTime, Duration, Utc};
use serde::Deserialize;
use serde_json::json;
use sqlx::SqlitePool;
use tauri::{AppHandle, Emitter, Manager, Runtime};
use url::Url;

use super::events_map::{gcal_row_id, map_event, EventsPage};
use super::photos::fetch_attendee_photos;
use super::{cloud_identity, oauth};
use crate::calendar::eventkit::{Attendee, UpcomingMeeting};
use crate::database::repositories::attendee_photos::AttendeePhotosRepository;
use crate::database::repositories::google_calendar::{
    GoogleCalendarEventRow, GoogleCalendarRepository, GoogleCalendarSyncRow,
};
use crate::database::repositories::owner_emails::OwnerEmailsRepository;
use crate::secrets;
use crate::state::AppState;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Rust→frontend event emitted (no payload) when the stored grant was revoked
/// or expired (`invalid_grant` on refresh) — the UI shows the reconnect prompt.
pub const AUTH_REQUIRED_EVENT: &str = "google-calendar-auth-required";

const CALENDAR_LIST_URL: &str = "https://www.googleapis.com/calendar/v3/users/me/calendarList";
const EVENTS_BASE_URL: &str = "https://www.googleapis.com/calendar/v3/calendars";

/// Agenda/upcoming builds trigger a sync when the cache is older than this.
const STALE_AFTER_SECS: i64 = 5 * 60;
/// Fixed background sync cadence while the app runs (owner decision 2026-07-02).
const BACKGROUND_SYNC_INTERVAL: std::time::Duration = std::time::Duration::from_secs(10 * 60);

/// Full-sync window: 14 days back (covers the agenda's recording-matching
/// lookback) to 60 days forward (upcoming/pick-list uses).
const WINDOW_LOOKBACK_DAYS: i64 = 14;
const WINDOW_HORIZON_DAYS: i64 = 60;
/// Full-resync early, while `now` is still this far from the window's far edge.
const HORIZON_SLACK_DAYS: i64 = 7;

const MAX_RESULTS: &str = "250";

/// Per-request HTTP bound so a dead network can't stall an agenda build that
/// awaited [`sync_if_stale`] (reqwest's default is *no* timeout).
const HTTP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

/// ± window for the title+time attendee fallback — mirrors the EventKit
/// fallback's 4-hour search window (`eventkit::read_event_attendees`).
const TITLE_MATCH_WINDOW_SECS: i64 = 4 * 3600;

// ---------------------------------------------------------------------------
// Single-flight + auth-required latch
// ---------------------------------------------------------------------------

/// Single-flight guard: concurrent triggers (focus + timer + agenda build)
/// coalesce — whoever holds the lock syncs, everyone else no-ops.
static SYNC_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Latched when a refresh came back `invalid_grant`, so sync passes stop
/// hitting the network (and stop re-emitting the event) until the user
/// reconnects or disconnects.
static AUTH_REQUIRED: AtomicBool = AtomicBool::new(false);

/// Clear the `invalid_grant` latch (connect/disconnect paths).
pub fn clear_auth_required() {
    AUTH_REQUIRED.store(false, Ordering::SeqCst);
}

// ---------------------------------------------------------------------------
// Public triggers
// ---------------------------------------------------------------------------

/// The app's DB pool, or `None` before database init (first launch) — every
/// Google Calendar path degrades to a no-op/cache-miss without it.
pub(crate) fn db_pool<R: Runtime>(app: &AppHandle<R>) -> Option<SqlitePool> {
    app.try_state::<AppState>()
        .map(|s| s.db_manager.pool().clone())
}

/// Sync every selected calendar of the connected account. No-ops (Ok) when the
/// feature isn't configured, no account is connected, a sync is already in
/// flight (single-flight coalescing), or the auth-required latch is set.
///
/// Errors are transient (offline/quota/API) unless they came from the token
/// refresh; `invalid_grant` emits [`AUTH_REQUIRED_EVENT`] once, latches
/// further passes off, and returns the user-actionable auth error.
pub async fn sync_all<R: Runtime>(app: &AppHandle<R>) -> Result<()> {
    if !super::is_configured() {
        return Ok(());
    }
    let Ok(_guard) = SYNC_LOCK.try_lock() else {
        log::debug!("google calendar: sync already in flight; coalescing trigger");
        return Ok(());
    };
    let Some(pool) = db_pool(app) else {
        return Ok(());
    };
    if GoogleCalendarRepository::get_account(&pool)
        .await
        .context("could not read the Google Calendar account state")?
        .is_none()
    {
        return Ok(()); // not connected — nothing to sync
    }
    if AUTH_REQUIRED.load(Ordering::SeqCst) {
        return Ok(()); // revoked grant already surfaced; wait for reconnect
    }

    let Some(store) = secrets::store() else {
        bail!("the macOS Keychain is unavailable, so the Google Calendar connection can't be used");
    };
    let token = match oauth::get_access_token(store).await {
        Ok(token) => token,
        Err(e) if oauth::is_invalid_grant(&e) => {
            // Emit once, then latch off — no error loop (spec 0032 failure modes).
            if !AUTH_REQUIRED.swap(true, Ordering::SeqCst) {
                log::warn!(
                    "google calendar: access was revoked or expired; emitting {AUTH_REQUIRED_EVENT}"
                );
                if let Err(emit_err) = app.emit(AUTH_REQUIRED_EVENT, ()) {
                    log::error!(
                        "google calendar: could not emit {AUTH_REQUIRED_EVENT}: {emit_err}"
                    );
                }
            }
            return Err(e);
        }
        Err(e) => return Err(e),
    };

    let client = http_client()?;
    // Refresh the calendar list (discovers new calendars, refreshes names —
    // selection and sync tokens are preserved by the upsert).
    sync_calendar_list(&client, &token, &pool).await?;

    let states = GoogleCalendarRepository::list_sync_states(&pool)
        .await
        .context("could not read the calendar sync state")?;
    let now = Utc::now();
    let fetch = |req: EventsRequest| http_fetch_events(&client, &token, req);

    // Probed enrichment capabilities (a granted scope is NOT access) — read BEFORE
    // the sync loop so DL detection can widen (RC-1) at map time. `widen` is on
    // ONLY when Cloud Identity can authoritatively confirm/correct a guess, so the
    // broadened, speculative classification never runs on the non-expandable path.
    let capabilities = GoogleCalendarRepository::get_capabilities(&pool)
        .await
        .ok()
        .flatten();
    let widen = matches!(
        capabilities.as_ref().and_then(|c| c.can_expand_groups),
        Some(true)
    );

    let mut first_err: Option<anyhow::Error> = None;
    for cal in states.iter().filter(|s| s.selected) {
        if let Err(e) = sync_calendar(&pool, cal, now, widen, &fetch).await {
            log::warn!(
                "google calendar: sync of calendar '{}' failed (serving cache): {e:#}",
                cal.calendar_id
            );
            first_err.get_or_insert(e);
        }
    }

    // Best-effort enrichment passes (specs/0038 WS3), each gated on its own
    // probed capability. Both run after the cache is refreshed so they see every
    // attendee, and both are wholly non-fatal — a failure never touches the sync
    // result.

    // DL flattening: only when the org granted Cloud Identity access. One
    // group's failure never aborts the pass; the labeled-DL floor stands.
    if matches!(
        capabilities.as_ref().and_then(|c| c.can_expand_groups),
        Some(true)
    ) {
        let owner_emails: std::collections::HashSet<String> = OwnerEmailsRepository::list(&pool)
            .await
            .unwrap_or_default()
            .into_iter()
            .collect();
        if let Err(e) = expand_distribution_lists(&pool, &client, &token, &owner_emails).await {
            log::warn!("google calendar: DL expansion pass failed (floor stands): {e:#}");
        }
    }

    // Attendee photos: only when the org granted the People directory read.
    // Downloads same-org photos into the local cache as base64 data: URIs; any
    // failure degrades to initials and never touches the sync result.
    if matches!(
        capabilities.as_ref().and_then(|c| c.can_fetch_photos),
        Some(true)
    ) {
        if let Err(e) = fetch_attendee_photos(&pool, &client, &token).await {
            log::warn!("google calendar: attendee photo pass failed (initials stand): {e:#}");
        }
    }

    match first_err {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

/// Best-effort staleness-gated sync: runs [`sync_all`] only when the newest
/// `last_synced_at` over the selected calendars is older than 5 minutes (or
/// absent). Never returns an error — agenda/upcoming builds and the background
/// timer call this and must keep serving the cache on any failure.
pub async fn sync_if_stale<R: Runtime>(app: &AppHandle<R>) {
    if !super::is_configured() {
        return;
    }
    let Some(pool) = db_pool(app) else {
        return;
    };
    // Instant no-op when not connected (the background timer's steady state).
    match GoogleCalendarRepository::get_account(&pool).await {
        Ok(Some(_)) => {}
        _ => return,
    }
    let states = GoogleCalendarRepository::list_sync_states(&pool)
        .await
        .unwrap_or_default();
    let newest = states
        .iter()
        .filter(|s| s.selected)
        .filter_map(|s| s.last_synced_at.as_deref())
        .filter_map(|t| t.parse::<DateTime<Utc>>().ok())
        .max();
    let stale = newest
        .map(|t| Utc::now() - t >= Duration::seconds(STALE_AFTER_SECS))
        .unwrap_or(true);
    if !stale {
        return;
    }
    if let Err(e) = sync_all(app).await {
        log::warn!("google calendar: staleness-triggered sync failed (serving cache): {e:#}");
    }
}

/// Spawn the fixed 10-minute background sync timer (spec 0032, owner decision
/// 2026-07-02). Called once from the app's setup hook. Each tick is a
/// [`sync_if_stale`] — an instant no-op while no account is connected.
pub fn spawn_background_sync<R: Runtime>(app: AppHandle<R>) {
    if !super::is_configured() {
        return; // feature inert in no-Google builds (acceptance criterion 9)
    }
    tauri::async_runtime::spawn(async move {
        // One-time startup backfill before the timer takes over: accounts
        // connected before the connect-time owner-email hook existed never had
        // their address recorded — ensure it now (idempotent, best-effort).
        backfill_owner_email_for_connected_account(&app).await;

        // Capability probe for already-connected accounts (specs/0038 WS3):
        // accounts connected before the probe existed have no capability flags,
        // so probe once here. Stale-gated (>7 days) so it's a no-op on already
        // fresh accounts, and fully best-effort (never blocks the sync timer).
        super::capabilities::probe_if_stale(&app).await;

        let mut interval = tokio::time::interval(BACKGROUND_SYNC_INTERVAL);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        interval.tick().await; // consume the immediate first tick — the first
                               // agenda build handles startup freshness
        loop {
            interval.tick().await;
            sync_if_stale(&app).await;
        }
    });
}

/// specs/0018 + 0032: the connected Google account's email is authoritative
/// "this is me" evidence, so it belongs in `owner_emails` for participant
/// seeding. `api_google_calendar_connect` adds it at connect time; this
/// startup pass covers accounts connected BEFORE that hook existed.
/// `OwnerEmailsRepository::add` is idempotent (INSERT OR IGNORE on the
/// normalized address), so re-running every launch is a silent no-op.
/// Best-effort by contract: any failure logs a warning and never blocks the
/// sync timer. Deliberately never removed on disconnect — the address is
/// still the user's; owner emails stay user-managed in Settings.
async fn backfill_owner_email_for_connected_account<R: Runtime>(app: &AppHandle<R>) {
    let Some(pool) = db_pool(app) else {
        return;
    };
    match GoogleCalendarRepository::get_account(&pool).await {
        Ok(Some(account)) => {
            if let Err(e) = OwnerEmailsRepository::add(&pool, &account.email).await {
                log::warn!(
                    "google calendar: could not record the connected account as an owner email: {e}"
                );
            }
        }
        Ok(None) => {} // not connected — nothing to backfill
        Err(e) => {
            log::warn!(
                "google calendar: owner-email startup backfill could not read the account: {e}"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Cache readers (the active-source agenda/upcoming feed + attendee routing)
// ---------------------------------------------------------------------------

/// Convert a cached Google event row into the wire [`UpcomingMeeting`] shape
/// the whole downstream pipeline consumes: `id` keeps the routing-discriminator
/// `gcal:` prefix (adoption/attendee lookups treat it as opaque), and
/// `external_id` carries the iCalUID so `stable_dismiss_key` produces the exact
/// same `ext:<uid>@<start>` key an EventKit read of the same event would —
/// dismissals survive switching the active source (see the test below).
fn to_upcoming_meeting(
    row: &GoogleCalendarEventRow,
    calendar_name: Option<&str>,
) -> UpcomingMeeting {
    UpcomingMeeting {
        id: row.id.clone(),
        title: row.title.clone(),
        starts_at: row.starts_at.clone(),
        ends_at: row.ends_at.clone(),
        calendar_name: calendar_name.unwrap_or("Google Calendar").to_string(),
        location: row.location.clone(),
        zoom_url: row.zoom_url.clone(),
        external_id: row.ical_uid.clone(),
    }
}

/// Cached Google events starting in `[start, end)`, as [`UpcomingMeeting`]s —
/// THE calendar feed while a Google account is connected (specs/0032,
/// single-active-source model). Duplicate rows of the SAME invite on multiple
/// synced calendars are collapsed by iCalUID + start (specs/0041 WS5) before
/// mapping. Best-effort: any DB error returns an empty list (a recordings-only
/// agenda), never an error.
pub async fn cached_upcoming_between(
    pool: &SqlitePool,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) -> Vec<UpcomingMeeting> {
    let names: std::collections::HashMap<String, String> =
        match GoogleCalendarRepository::list_sync_states(pool).await {
            Ok(states) => states
                .into_iter()
                .map(|s| (s.calendar_id, s.summary))
                .collect(),
            Err(e) => {
                log::warn!("google calendar: could not read calendar names: {e}");
                Default::default()
            }
        };
    let rows = match GoogleCalendarRepository::events_between(
        pool,
        &start.to_rfc3339(),
        &end.to_rfc3339(),
    )
    .await
    {
        Ok(rows) => rows,
        Err(e) => {
            log::warn!("google calendar: cached event read failed (skipping merge): {e}");
            return Vec::new();
        }
    };
    // The account's primary calendar id IS the account email (Google's model) —
    // the tie-break preference for which duplicate row survives the collapse.
    let primary_calendar_id = GoogleCalendarRepository::get_account(pool)
        .await
        .ok()
        .flatten()
        .map(|a| a.email);
    let rows = dedup_rows_by_ical_uid(rows, primary_calendar_id.as_deref());
    rows.iter()
        .map(|row| to_upcoming_meeting(row, names.get(&row.calendar_id).map(String::as_str)))
        .collect()
}

/// Collapse cached rows of the SAME invite that appear on several synced
/// calendars (specs/0041 WS5): rows sharing an `ical_uid` AND `starts_at` are
/// one event on Google's side, cached once per calendar as
/// `gcal:{calendar_id}/{event_id}`. Survivor preference:
///   1. the organizer's own calendar (`calendar_id == organizer_email`),
///   2. the account's primary calendar,
///   3. else the first row encountered (rows arrive ordered by start).
///
/// Rows without an `ical_uid` are never collapsed. Recurring occurrences share
/// a UID but differ in `starts_at`, so they all survive. Dismissals are keyed
/// `ext:{iCalUID}@{starts_at}` — identical for every duplicate — so a stored
/// dismissal applies to whichever row survives (asserted in tests).
fn dedup_rows_by_ical_uid(
    rows: Vec<GoogleCalendarEventRow>,
    primary_calendar_id: Option<&str>,
) -> Vec<GoogleCalendarEventRow> {
    /// Lower ranks are preferred survivors.
    fn rank(row: &GoogleCalendarEventRow, primary: Option<&str>) -> u8 {
        let is_organizers_calendar = row
            .organizer_email
            .as_deref()
            .is_some_and(|org| org.eq_ignore_ascii_case(&row.calendar_id));
        if is_organizers_calendar {
            0
        } else if primary.is_some_and(|p| p.eq_ignore_ascii_case(&row.calendar_id)) {
            1
        } else {
            2
        }
    }

    let mut out: Vec<GoogleCalendarEventRow> = Vec::with_capacity(rows.len());
    // (ical_uid, starts_at) → index into `out` of the current survivor.
    let mut seen: std::collections::HashMap<(String, String), usize> =
        std::collections::HashMap::new();
    for row in rows {
        let Some(uid) = row.ical_uid.clone().filter(|u| !u.trim().is_empty()) else {
            out.push(row);
            continue;
        };
        match seen.entry((uid, row.starts_at.clone())) {
            std::collections::hash_map::Entry::Vacant(slot) => {
                slot.insert(out.len());
                out.push(row);
            }
            std::collections::hash_map::Entry::Occupied(slot) => {
                let kept = &mut out[*slot.get()];
                if rank(&row, primary_calendar_id) < rank(kept, primary_calendar_id) {
                    // Replace in place: keeps the first occurrence's position,
                    // so the list stays ordered by start.
                    *kept = row;
                }
            }
        }
    }
    out
}

/// Attendees of one cached event by its `gcal:` id — the `gcal:`-prefix
/// routing target for `event_attendees_by_id` consumers (spec 0032). Empty on
/// miss/error (parity with the EventKit reader's degrade-to-empty contract).
/// The miss case includes the post-disconnect path: `purge_all` empties the
/// cache, so a meeting recorded under a `gcal:` id degrades to an empty
/// attendee list after the user disconnects — never an error.
pub async fn cached_attendees_for_event_id(pool: &SqlitePool, gcal_id: &str) -> Vec<Attendee> {
    cached_attendees_for_event_id_with_photos(pool, gcal_id, &cached_photo_map(pool).await).await
}

/// As [`cached_attendees_for_event_id`], but taking a pre-loaded photo map so a
/// list/agenda build that resolves many events reads the photo table ONCE
/// instead of per-event (specs/0038 WS3, finding #9). The single-event callers
/// use the wrapper above.
pub async fn cached_attendees_for_event_id_with_photos(
    pool: &SqlitePool,
    gcal_id: &str,
    photos: &std::collections::HashMap<String, String>,
) -> Vec<Attendee> {
    match GoogleCalendarRepository::get_event(pool, gcal_id).await {
        Ok(Some(row)) => attendees_from_json(&row.attendees_json, photos),
        Ok(None) => {
            log::info!("google calendar: no cached event for id '{gcal_id}'");
            Vec::new()
        }
        Err(e) => {
            log::warn!("google calendar: cached attendee read failed for '{gcal_id}': {e}");
            Vec::new()
        }
    }
}

/// Title+time attendee fallback over the Google cache — the same shape as
/// `eventkit::event_attendees` (±4h window around the meeting start, trimmed
/// case-insensitive title match, closest start wins). Used when a meeting has
/// no persisted calendar event id and EventKit found nothing.
pub async fn cached_attendees_by_title_time(
    pool: &SqlitePool,
    title: &str,
    started_at: &str,
) -> Vec<Attendee> {
    let Ok(started) = started_at.parse::<DateTime<Utc>>() else {
        log::warn!("google calendar: invalid meeting start instant '{started_at}'");
        return Vec::new();
    };
    let window = Duration::seconds(TITLE_MATCH_WINDOW_SECS);
    let rows = match GoogleCalendarRepository::events_between(
        pool,
        &(started - window).to_rfc3339(),
        &(started + window).to_rfc3339(),
    )
    .await
    {
        Ok(rows) => rows,
        Err(e) => {
            log::warn!("google calendar: cached title+time attendee lookup failed: {e}");
            return Vec::new();
        }
    };
    let want = title.trim().to_lowercase();
    let best = rows
        .iter()
        .filter(|r| r.title.trim().to_lowercase() == want)
        .filter_map(|r| {
            let start = r.starts_at.parse::<DateTime<Utc>>().ok()?;
            Some(((start - started).num_seconds().abs(), r))
        })
        .min_by_key(|(delta, _)| *delta)
        .map(|(_, row)| row);
    match best {
        Some(row) => attendees_from_json(&row.attendees_json, &cached_photo_map(pool).await),
        None => Vec::new(),
    }
}

/// The cached attendee-photo map (`normalized email → data: URI`), or an empty
/// map on any DB error (specs/0038 WS3). Best-effort by contract — a photo
/// lookup failure just means initials, never a broken attendee list. Load it
/// ONCE per list/agenda build and thread it into the `_with_photos` readers so
/// the photo table isn't full-scanned per event (finding #9).
pub(crate) async fn cached_photo_map(
    pool: &SqlitePool,
) -> std::collections::HashMap<String, String> {
    match AttendeePhotosRepository::all_photos(pool).await {
        Ok(map) => map,
        Err(e) => {
            log::warn!("google calendar: could not read cached attendee photos: {e}");
            std::collections::HashMap::new()
        }
    }
}

/// Parse a cache row's `attendees_json` into the wire [`Attendee`] shape the
/// EventKit path returns (name/email/isCurrentUser — the cached
/// `responseStatus`/`isOrganizer` fields stay in the row for future use).
///
/// `photos` is the cached `lowercased email → data: URI` map (specs/0038 WS3);
/// each attendee's photo is looked up by email so the frontend can render a face
/// with no render-time egress. An empty map (the common case: photos never
/// fetched, or org forbids the People directory) leaves every `photo_data_uri`
/// `None`, which the UI degrades to initials.
pub fn attendees_from_json(
    attendees_json: &str,
    photos: &std::collections::HashMap<String, String>,
) -> Vec<Attendee> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct CachedAttendee {
        name: Option<String>,
        email: Option<String>,
        #[serde(default)]
        is_current_user: bool,
        #[serde(default)]
        is_distribution_list: bool,
    }
    let parsed: Vec<CachedAttendee> = match serde_json::from_str(attendees_json) {
        Ok(parsed) => parsed,
        Err(e) => {
            log::warn!("google calendar: malformed cached attendees_json: {e}");
            return Vec::new();
        }
    };
    parsed
        .into_iter()
        .map(|a| {
            let photo_data_uri = a
                .email
                .as_deref()
                .map(|e| e.trim().to_ascii_lowercase())
                .filter(|e| !e.is_empty())
                .and_then(|e| photos.get(&e).cloned());
            Attendee {
                name: a
                    .name
                    .filter(|n| !n.trim().is_empty())
                    .or_else(|| a.email.clone())
                    .unwrap_or_else(|| "Unknown".to_string()),
                email: a.email,
                is_current_user: a.is_current_user,
                is_distribution_list: a.is_distribution_list,
                photo_data_uri,
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Calendar-list sync (account identity + per-calendar sync states)
// ---------------------------------------------------------------------------

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct CalendarListPage {
    items: Vec<CalendarListItem>,
    next_page_token: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct CalendarListItem {
    id: String,
    summary: Option<String>,
    summary_override: Option<String>,
    primary: bool,
}

/// Accounts exposing MORE than this many calendars register NEW calendars as
/// unselected (except the primary), so connecting a large account doesn't sync
/// ~30 calendars by default (specs/0041 WS5, owner rule 2026-07-08).
const AUTO_SELECT_MAX_CALENDARS: usize = 5;

/// Fetch the account's calendar list, upsert every calendar's sync state
/// (names refresh; selection/sync tokens are preserved), and return the
/// `primary` calendar's id — which for Google IS the account email, the
/// identity shown in Settings (spec 0032 connect flow).
pub(crate) async fn sync_calendar_list(
    client: &reqwest::Client,
    token: &str,
    pool: &SqlitePool,
) -> Result<Option<String>> {
    // Collect ALL pages first: the selection default for new rows depends on the
    // account's total calendar count (see `register_calendar_list`).
    let mut items: Vec<CalendarListItem> = Vec::new();
    let mut page_token: Option<String> = None;
    loop {
        let mut url = Url::parse(CALENDAR_LIST_URL).context("invalid calendarList endpoint")?;
        if let Some(t) = &page_token {
            url.query_pairs_mut().append_pair("pageToken", t);
        }
        let resp = client
            .get(url)
            .bearer_auth(token)
            .send()
            .await
            .context("could not reach Google Calendar (calendar list)")?;
        let status = resp.status();
        if !status.is_success() {
            bail!("Google calendar-list request failed (HTTP {status})");
        }
        let page: CalendarListPage = resp
            .json()
            .await
            .context("unexpected calendarList response from Google")?;
        items.extend(page.items);
        match page.next_page_token {
            Some(t) => page_token = Some(t),
            None => break,
        }
    }
    register_calendar_list(pool, &items).await
}

/// Upsert the fetched calendar list into `google_calendar_sync` and return the
/// primary calendar's id. Split from the HTTP fetch so the selection-default
/// rule is unit-testable (specs/0041 WS5):
///   - ≤ [`AUTO_SELECT_MAX_CALENDARS`] calendars ⇒ NEW rows default selected
///     (the original all-selected behavior);
///   - more ⇒ NEW rows default unselected, except the `primary` calendar.
///
/// Existing rows always keep the user's selection (the upsert only refreshes
/// the display name), so re-syncing the list never clobbers choices.
async fn register_calendar_list(
    pool: &SqlitePool,
    items: &[CalendarListItem],
) -> Result<Option<String>> {
    let valid: Vec<&CalendarListItem> = items.iter().filter(|i| !i.id.trim().is_empty()).collect();
    let select_all_by_default = valid.len() <= AUTO_SELECT_MAX_CALENDARS;

    let mut primary: Option<String> = None;
    for item in valid {
        let summary = item
            .summary_override
            .as_deref()
            .or(item.summary.as_deref())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or(&item.id);
        GoogleCalendarRepository::upsert_sync_state(
            pool,
            &item.id,
            summary,
            select_all_by_default || item.primary,
        )
        .await
        .context("could not save the calendar list")?;
        if item.primary {
            primary = Some(item.id.clone());
        }
    }
    Ok(primary)
}

// ---------------------------------------------------------------------------
// events.list — request/response model + injectable HTTP layer
// ---------------------------------------------------------------------------

/// One `events.list` request, described declaratively so the HTTP layer is a
/// swappable function (tests feed canned JSON; production hits Google).
#[derive(Debug, Clone, PartialEq, Eq)]
enum EventsRequest {
    /// Windowed full sync (`singleEvents=true`, `timeMin`/`timeMax`, paged).
    Full {
        calendar_id: String,
        time_min: String,
        time_max: String,
        page_token: Option<String>,
    },
    /// Incremental sync (`syncToken` only, paged).
    Incremental {
        calendar_id: String,
        sync_token: String,
        page_token: Option<String>,
    },
}

/// What the HTTP layer hands back: a raw JSON body, or the distinct `410 GONE`
/// signal (expired syncToken ⇒ transparent full resync).
enum FetchBody {
    Json(String),
    Gone,
}

pub(crate) fn http_client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(HTTP_TIMEOUT)
        .build()
        .context("could not build the Google Calendar HTTP client")
}

fn events_url(req: &EventsRequest) -> Result<Url> {
    let calendar_id = match req {
        EventsRequest::Full { calendar_id, .. }
        | EventsRequest::Incremental { calendar_id, .. } => calendar_id,
    };
    let mut url = Url::parse(EVENTS_BASE_URL).context("invalid events endpoint")?;
    url.path_segments_mut()
        .map_err(|_| anyhow!("invalid events endpoint base"))?
        .push(calendar_id) // percent-encodes the '@' in calendar ids
        .push("events");
    match req {
        EventsRequest::Full {
            time_min,
            time_max,
            page_token,
            ..
        } => {
            url.query_pairs_mut()
                .append_pair("singleEvents", "true")
                .append_pair("timeMin", time_min)
                .append_pair("timeMax", time_max)
                .append_pair("maxResults", MAX_RESULTS);
            if let Some(t) = page_token {
                url.query_pairs_mut().append_pair("pageToken", t);
            }
        }
        EventsRequest::Incremental {
            sync_token,
            page_token,
            ..
        } => {
            url.query_pairs_mut().append_pair("syncToken", sync_token);
            if let Some(t) = page_token {
                url.query_pairs_mut().append_pair("pageToken", t);
            }
        }
    }
    Ok(url)
}

/// The production HTTP layer for [`sync_calendar`]. `403/429` (quota) become
/// plain errors — the trigger cadence is the retry/backoff for v1 (spec 0032
/// failure modes; documented tradeoff, no in-process retry storm).
async fn http_fetch_events(
    client: &reqwest::Client,
    token: &str,
    req: EventsRequest,
) -> Result<FetchBody> {
    let url = events_url(&req)?;
    let resp = client
        .get(url)
        .bearer_auth(token)
        .send()
        .await
        .context("could not reach Google Calendar (events)")?;
    match resp.status().as_u16() {
        410 => Ok(FetchBody::Gone),
        403 | 429 => bail!(
            "Google Calendar API returned HTTP {} (quota/permission); \
             the sync will retry on the next trigger",
            resp.status().as_u16()
        ),
        s if !resp.status().is_success() => {
            bail!("Google Calendar events request failed (HTTP {s})")
        }
        _ => Ok(FetchBody::Json(resp.text().await.context(
            "could not read the Google Calendar events response",
        )?)),
    }
}

// ---------------------------------------------------------------------------
// Distribution-list expansion (optimistic; specs/0038 WS3)
// ---------------------------------------------------------------------------

/// What Cloud Identity said about one DL-flagged address (specs/0038 WS3,
/// findings #1/#8).
#[derive(Debug, Clone)]
enum DlResolution {
    /// The address IS a group and its members were listed (possibly empty) —
    /// fold the members in and mark the DL attendee `expanded` so a steady-state
    /// incremental sync doesn't re-list it every pass (finding #8).
    Expanded(Vec<cloud_identity::GroupMember>),
    /// A definitive `404`: the address is NOT a group, so the heuristic
    /// false-positived — clear the DL flag so it seeds as a person (finding #1).
    NotAGroup,
    /// Denied (`403`) or a transient error — inconclusive. Keep the labeled-DL
    /// floor untouched and retry on a later pass.
    Retry,
}

/// How to annotate a DL attendee in place after resolution.
enum DlMark {
    /// Set `expanded: true` (a real group whose members were folded in).
    Expanded,
    /// Set `isDistributionList: false` (a false positive — it's a person).
    ClearFlag,
}

/// Fold Cloud Identity group members into cached events that carry a DL
/// attendee, when the org grants access (`can_expand_groups`). Best-effort and
/// idempotent: each address is resolved once per pass (cached in `resolved`),
/// members are deduped against the event's existing attendees and the owner
/// emails, and a `Denied`/error leaves the labeled-DL floor untouched. One
/// address's failure never aborts the pass or the sync.
///
/// The heuristic is a label-only floor that this pass CONFIRMS or CORRECTS:
///   - a confirmed group is expanded and its DL attendee marked `expanded`, so
///     an unchanged event makes ZERO Cloud Identity calls on later syncs (#8);
///   - a confirmed non-group has its `isDistributionList` flag CLEARED, so a
///     real person the heuristic misflagged seeds normally and stops being
///     re-queried (finding #1).
async fn expand_distribution_lists(
    pool: &SqlitePool,
    client: &reqwest::Client,
    token: &str,
    owner_emails: &std::collections::HashSet<String>,
) -> Result<()> {
    use crate::database::repositories::owner_emails::normalize_email;

    let rows = GoogleCalendarRepository::events_with_distribution_lists(pool)
        .await
        .context("could not list events with distribution lists")?;

    // Normalized address -> its resolution, resolved once per pass (coalesces the
    // same DL invited to many meetings into one Cloud Identity round-trip).
    let mut resolved: std::collections::HashMap<String, DlResolution> =
        std::collections::HashMap::new();

    for row in rows {
        let mut attendees: Vec<serde_json::Value> = match serde_json::from_str(&row.attendees_json)
        {
            Ok(v) => v,
            Err(e) => {
                log::warn!(
                    "google calendar: DL expansion skipped malformed attendees for '{}': {e}",
                    row.id
                );
                continue;
            }
        };

        // Existing (normalized) emails so folded members don't duplicate a
        // person Calendar already materialized, the owner, or each other.
        let mut present: std::collections::HashSet<String> = attendees
            .iter()
            .filter_map(|a| a.get("email").and_then(|v| v.as_str()))
            .map(normalize_email)
            .filter(|e| !e.is_empty())
            .collect();
        present.extend(owner_emails.iter().cloned());

        // UNEXPANDED DL addresses on this event: a DL already marked `expanded`
        // was folded on a previous pass, so skip it — an unchanged event makes
        // no Cloud Identity calls (finding #8). An incremental sync that rewrote
        // this event's attendees drops the marker, so real changes re-expand.
        let dl_emails: Vec<String> = attendees
            .iter()
            .filter(|a| a.get("isDistributionList").and_then(|v| v.as_bool()) == Some(true))
            .filter(|a| a.get("expanded").and_then(|v| v.as_bool()) != Some(true))
            .filter_map(|a| a.get("email").and_then(|v| v.as_str()))
            .map(str::trim)
            .filter(|e| !e.is_empty())
            .map(|e| e.to_string())
            .collect();
        if dl_emails.is_empty() {
            continue;
        }

        let mut changed = false;
        for dl_email in dl_emails {
            let key = normalize_email(&dl_email);
            if key.is_empty() {
                continue;
            }
            if !resolved.contains_key(&key) {
                let outcome = resolve_distribution_list(client, token, &dl_email).await;
                resolved.insert(key.clone(), outcome);
            }
            match resolved.get(&key).cloned().unwrap_or(DlResolution::Retry) {
                DlResolution::Expanded(members) => {
                    let mut added = 0usize;
                    for member in members {
                        let Some(member_email) = member.email.as_deref() else {
                            continue;
                        };
                        let norm = normalize_email(member_email);
                        if norm.is_empty() || present.contains(&norm) {
                            continue;
                        }
                        present.insert(norm);
                        let name = member
                            .display_name
                            .as_deref()
                            .map(str::trim)
                            .filter(|n| !n.is_empty())
                            .map(str::to_string)
                            .unwrap_or_else(|| member_email.to_string());
                        attendees.push(json!({
                            "name": name,
                            "email": member_email,
                            "isCurrentUser": false,
                            "responseStatus": serde_json::Value::Null,
                            "isOrganizer": false,
                            "isDistributionList": false,
                        }));
                        added += 1;
                    }
                    // Mark the DL attendee expanded so it isn't re-listed next
                    // pass (even a zero-member group is "done" — don't retry it).
                    changed |= mark_distribution_list(&mut attendees, &key, DlMark::Expanded);
                    if added > 0 {
                        log::info!(
                            "google calendar: folded {added} DL member(s) into '{}'",
                            row.id
                        );
                        changed = true;
                    }
                }
                DlResolution::NotAGroup => {
                    if mark_distribution_list(&mut attendees, &key, DlMark::ClearFlag) {
                        log::info!(
                            "google calendar: '{}' is not a group; clearing the DL flag so it seeds as a person",
                            redact_email(&key)
                        );
                        changed = true;
                    }
                }
                DlResolution::Retry => {} // inconclusive — keep the floor, retry later
            }
        }

        if changed {
            let json = serde_json::to_string(&attendees).unwrap_or(row.attendees_json);
            if let Err(e) =
                GoogleCalendarRepository::set_event_attendees_json(pool, &row.id, &json).await
            {
                log::warn!(
                    "google calendar: could not save expanded attendees for '{}': {e}",
                    row.id
                );
            }
        }
    }
    Ok(())
}

/// Resolve one DL-flagged address against Cloud Identity, collapsing every
/// best-effort non-outcome into a [`DlResolution`] (never returns an error).
async fn resolve_distribution_list(
    client: &reqwest::Client,
    token: &str,
    email: &str,
) -> DlResolution {
    match cloud_identity::lookup_group(client, token, email).await {
        Ok(cloud_identity::LookupOutcome::Group(group_name)) => {
            match cloud_identity::list_members(client, token, &group_name).await {
                Ok(cloud_identity::MembersOutcome::Members(members)) => {
                    DlResolution::Expanded(members)
                }
                Ok(cloud_identity::MembersOutcome::Denied) => {
                    log::info!("google calendar: a DL's member listing was denied; keeping floor");
                    DlResolution::Retry
                }
                Err(e) => {
                    log::warn!("google calendar: a DL's member listing failed: {e:#}");
                    DlResolution::Retry
                }
            }
        }
        // 404: definitively not a group → the heuristic false-positived.
        Ok(cloud_identity::LookupOutcome::NotAGroup) => DlResolution::NotAGroup,
        // 403: not allowed to look it up → inconclusive, never demote a group
        // we simply can't see.
        Ok(cloud_identity::LookupOutcome::Denied) => DlResolution::Retry,
        Err(e) => {
            log::warn!("google calendar: a DL lookup failed: {e:#}");
            DlResolution::Retry
        }
    }
}

/// Annotate the DL-flagged attendee whose (normalized) email is `key` in place.
/// Returns whether the attendee list actually changed (idempotent — re-marking
/// an already-marked attendee is a no-op that returns `false`).
fn mark_distribution_list(attendees: &mut [serde_json::Value], key: &str, mark: DlMark) -> bool {
    use crate::database::repositories::owner_emails::normalize_email;
    for a in attendees.iter_mut() {
        if a.get("isDistributionList").and_then(|v| v.as_bool()) != Some(true) {
            continue;
        }
        let matches = a
            .get("email")
            .and_then(|v| v.as_str())
            .map(normalize_email)
            .as_deref()
            == Some(key);
        if !matches {
            continue;
        }
        let Some(obj) = a.as_object_mut() else {
            continue;
        };
        match mark {
            DlMark::Expanded => {
                if obj.get("expanded").and_then(|v| v.as_bool()) == Some(true) {
                    return false;
                }
                obj.insert("expanded".to_string(), json!(true));
            }
            DlMark::ClearFlag => {
                obj.insert("isDistributionList".to_string(), json!(false));
            }
        }
        return true;
    }
    false
}

/// Redact an email for logs (privacy, finding #10): keep the first local-part
/// character and the domain, mask the rest — enough to correlate entries
/// without writing the raw address to disk.
pub(super) fn redact_email(email: &str) -> String {
    match email.split_once('@') {
        Some((local, domain)) => {
            let first = local.chars().next().unwrap_or('*');
            format!("{first}***@{domain}")
        }
        None => "***".to_string(),
    }
}

// ---------------------------------------------------------------------------
// Per-calendar sync engine (HTTP-injected; unit-tested below)
// ---------------------------------------------------------------------------

/// The full/incremental window for one full sync at `now`.
fn sync_window(now: DateTime<Utc>) -> (String, String) {
    (
        (now - Duration::days(WINDOW_LOOKBACK_DAYS)).to_rfc3339(),
        (now + Duration::days(WINDOW_HORIZON_DAYS)).to_rfc3339(),
    )
}

/// True while the persisted full-sync horizon is still comfortably ahead of
/// `now` — incremental syncs stay valid. False (⇒ full resync re-extends the
/// window) when the horizon is unknown, unparseable, or within 7 days.
fn horizon_healthy(window_ends_at: Option<&str>, now: DateTime<Utc>) -> bool {
    window_ends_at
        .and_then(|h| h.parse::<DateTime<Utc>>().ok())
        .map(|h| now + Duration::days(HORIZON_SLACK_DAYS) < h)
        .unwrap_or(false)
}

/// Max days of unbroken incremental syncs before a full resync is forced
/// (specs/0054 W5).
///
/// [`horizon_healthy`] alone permits roughly 53 days between full syncs (a 60-day
/// window with 7 days of slack). A full sync is the only thing that REPLACES the
/// calendar's rows, so it is also the only thing that repairs cache drift — and
/// for 53 days at a time, nothing did. The W5 bug survived weeks on the
/// reporter's machine for exactly this reason.
const MAX_INCREMENTAL_DAYS: i64 = 7;

/// True while incremental syncs remain trustworthy: the window horizon is still
/// comfortably ahead AND the last pass is recent enough that drift cannot pile
/// up. Anything unknown or unparseable falls back to a full resync.
fn incremental_is_safe(
    window_ends_at: Option<&str>,
    last_synced_at: Option<&str>,
    now: DateTime<Utc>,
) -> bool {
    horizon_healthy(window_ends_at, now)
        && last_synced_at
            .and_then(|t| t.parse::<DateTime<Utc>>().ok())
            .is_some_and(|t| now - t < Duration::days(MAX_INCREMENTAL_DAYS))
}

/// Sync one calendar: incremental when a syncToken exists and the window
/// horizon is healthy; otherwise (no token / 410 / horizon near expiry) a full
/// windowed resync that replaces the calendar's cached rows (which is also the
/// out-of-window prune).
async fn sync_calendar<F, Fut>(
    pool: &SqlitePool,
    cal: &GoogleCalendarSyncRow,
    now: DateTime<Utc>,
    widen: bool,
    fetch: &F,
) -> Result<()>
where
    F: Fn(EventsRequest) -> Fut,
    Fut: Future<Output = Result<FetchBody>>,
{
    let incremental_token = cal.sync_token.as_deref().filter(|_| {
        incremental_is_safe(
            cal.window_ends_at.as_deref(),
            cal.last_synced_at.as_deref(),
            now,
        )
    });

    if let Some(token) = incremental_token {
        match incremental_sync(pool, &cal.calendar_id, token, widen, fetch).await? {
            Some(next_token) => {
                GoogleCalendarRepository::set_sync_progress(
                    pool,
                    &cal.calendar_id,
                    Some(&next_token),
                )
                .await
                .context("could not record the sync token")?;
                return Ok(());
            }
            None => {
                // 410 GONE: the token expired server-side — clear it and fall
                // through to a transparent full resync (spec failure modes).
                log::info!(
                    "google calendar: syncToken for '{}' expired (410); full resync",
                    cal.calendar_id
                );
                GoogleCalendarRepository::set_sync_progress(pool, &cal.calendar_id, None)
                    .await
                    .context("could not clear the expired sync token")?;
            }
        }
    }

    full_sync(pool, &cal.calendar_id, now, widen, fetch).await
}

/// One incremental pass. `Ok(Some(token))` = done, store the new syncToken;
/// `Ok(None)` = the server said 410 GONE (caller full-resyncs).
async fn incremental_sync<F, Fut>(
    pool: &SqlitePool,
    calendar_id: &str,
    sync_token: &str,
    widen: bool,
    fetch: &F,
) -> Result<Option<String>>
where
    F: Fn(EventsRequest) -> Fut,
    Fut: Future<Output = Result<FetchBody>>,
{
    let mut page_token: Option<String> = None;
    // Set when a recurring series master turned up: the page's remaining items are
    // still applied, then the caller full-resyncs (specs/0054 W5).
    let mut needs_full_resync = false;
    loop {
        let body = match fetch(EventsRequest::Incremental {
            calendar_id: calendar_id.to_string(),
            sync_token: sync_token.to_string(),
            page_token: page_token.clone(),
        })
        .await?
        {
            FetchBody::Gone => return Ok(None),
            FetchBody::Json(body) => body,
        };
        let page: EventsPage =
            serde_json::from_str(&body).context("unexpected events.list response from Google")?;
        for item in &page.items {
            // specs/0054 W5: the series-master check comes FIRST, including for
            // cancellations. A cancelled master still carries `recurrence`, and
            // deleting it by exact id would remove nothing — its occurrences are
            // keyed `<series>_<instanceTs>`. Both cases want the same thing: drop
            // everything cached for the series.
            if item.is_series_master() {
                let series = item.series_id();
                let removed =
                    GoogleCalendarRepository::delete_events_for_series(pool, calendar_id, series)
                        .await
                        .context("could not clear a recurring series from the cache")?;
                log::info!(
                    "google calendar: recurring series changed on '{}' ({} cached row(s) dropped, cancelled={}); scheduling a full resync",
                    calendar_id,
                    removed,
                    item.is_cancelled()
                );
                // `singleEvents=true` only expands on the FULL path, so the series
                // cannot be re-expanded from this page. Force a full resync.
                needs_full_resync = true;
            } else if item.is_cancelled() {
                // Cancelled instance ⇒ drop the cache row (idempotent).
                GoogleCalendarRepository::delete_event(pool, &gcal_row_id(calendar_id, &item.id))
                    .await
                    .context("could not remove a cancelled event")?;
            } else if let Some(row) = map_event(calendar_id, item, widen) {
                GoogleCalendarRepository::upsert_event(pool, &row)
                    .await
                    .context("could not save a changed event")?;
            } else {
                log::warn!(
                    "google calendar: skipping unmappable event '{}' (no usable start/end)",
                    item.id
                );
            }
        }
        if needs_full_resync {
            // Same contract as a 410 GONE: the caller full-resyncs, which replaces
            // the calendar's rows and re-expands every series properly.
            return Ok(None);
        }
        if let Some(next) = page.next_page_token {
            page_token = Some(next);
            continue;
        }
        return Ok(Some(page.next_sync_token.ok_or_else(|| {
            anyhow!("Google did not return a syncToken at the end of an incremental sync")
        })?));
    }
}

/// One full windowed sync: page through `[now-14d, now+60d)`, then REPLACE the
/// calendar's cached rows and persist the new syncToken + window horizon. The
/// replace is deliberate — it prunes rows that fell outside the re-extended
/// window (spec 0032 cache-growth note).
async fn full_sync<F, Fut>(
    pool: &SqlitePool,
    calendar_id: &str,
    now: DateTime<Utc>,
    widen: bool,
    fetch: &F,
) -> Result<()>
where
    F: Fn(EventsRequest) -> Fut,
    Fut: Future<Output = Result<FetchBody>>,
{
    let (time_min, time_max) = sync_window(now);
    let mut rows: Vec<GoogleCalendarEventRow> = Vec::new();
    let mut page_token: Option<String> = None;
    let sync_token = loop {
        let body = match fetch(EventsRequest::Full {
            calendar_id: calendar_id.to_string(),
            time_min: time_min.clone(),
            time_max: time_max.clone(),
            page_token: page_token.clone(),
        })
        .await?
        {
            // A 410 on a token-less windowed listing shouldn't happen; treat
            // it as a transient API error and let the next trigger retry.
            FetchBody::Gone => bail!("Google rejected the full event sync (unexpected HTTP 410)"),
            FetchBody::Json(body) => body,
        };
        let page: EventsPage =
            serde_json::from_str(&body).context("unexpected events.list response from Google")?;
        for item in &page.items {
            if item.is_cancelled() {
                continue; // defensive: showDeleted defaults to false
            }
            match map_event(calendar_id, item, widen) {
                Some(row) => rows.push(row),
                None => log::warn!(
                    "google calendar: skipping unmappable event '{}' (no usable start/end)",
                    item.id
                ),
            }
        }
        if let Some(next) = page.next_page_token {
            page_token = Some(next);
            continue;
        }
        break page.next_sync_token;
    };

    // Replace the calendar's cache. The moment between delete and refill is a
    // few local writes — invisible next to the network fetch above.
    GoogleCalendarRepository::delete_events_for_calendar(pool, calendar_id)
        .await
        .context("could not clear the calendar's cached events")?;
    for row in &rows {
        GoogleCalendarRepository::upsert_event(pool, row)
            .await
            .context("could not save a synced event")?;
    }
    if sync_token.is_none() {
        // Extremely unusual; store None so the next pass full-syncs again.
        log::warn!("google calendar: full sync of '{calendar_id}' returned no nextSyncToken");
    }
    GoogleCalendarRepository::set_sync_progress(pool, calendar_id, sync_token.as_deref())
        .await
        .context("could not record the sync token")?;
    GoogleCalendarRepository::set_window_horizon(pool, calendar_id, &time_max)
        .await
        .context("could not record the sync window")?;
    log::info!(
        "google calendar: full sync of '{calendar_id}' cached {} event(s)",
        rows.len()
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests — canned events.list JSON fixtures; NO network, NO Keychain.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    // Wire types + mapping now live in `events_map` (specs/0054 W5), along with
    // their own tests; these fixtures are shared back for the sync-level tests.
    use super::super::events_map::test_fixtures::{parse_fixture_event, ATTENDEE_EVENT};
    use chrono::TimeZone;
    use sqlx::sqlite::SqlitePoolOptions;
    use std::sync::Mutex;

    /// Fresh in-memory SQLite through the real migration set (one connection —
    /// each in-memory connection is a separate database).
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

    async fn registered_calendar(pool: &SqlitePool, id: &str) -> GoogleCalendarSyncRow {
        GoogleCalendarRepository::upsert_sync_state(pool, id, "Test Calendar", true)
            .await
            .unwrap();
        GoogleCalendarRepository::get_sync_state(pool, id)
            .await
            .unwrap()
            .unwrap()
    }

    /// Scripted fetch: pops canned responses in order while recording every
    /// request, so tests assert both the wire decisions and the DB effects.
    struct ScriptedFetch {
        responses: Mutex<Vec<Result<FetchBody>>>,
        requests: Mutex<Vec<EventsRequest>>,
    }

    impl ScriptedFetch {
        fn new(responses: Vec<Result<FetchBody>>) -> Self {
            Self {
                responses: Mutex::new(responses.into_iter().rev().collect()),
                requests: Mutex::new(Vec::new()),
            }
        }

        fn call(&self, req: EventsRequest) -> std::future::Ready<Result<FetchBody>> {
            self.requests.lock().unwrap().push(req);
            let next = self
                .responses
                .lock()
                .unwrap()
                .pop()
                .expect("test scripted more requests than responses");
            std::future::ready(next)
        }

        fn requests(&self) -> Vec<EventsRequest> {
            self.requests.lock().unwrap().clone()
        }
    }

    fn now() -> DateTime<Utc> {
        "2026-07-02T00:00:00+00:00".parse().unwrap()
    }

    fn json(body: &str) -> Result<FetchBody> {
        Ok(FetchBody::Json(body.to_string()))
    }

    // --- Fixtures ---------------------------------------------------------

    /// Page 1 of a full sync: a plain event + a recurring instance; non-ASCII
    /// title on the plain event; pages on.
    const FULL_PAGE_1: &str = r#"{
        "kind": "calendar#events",
        "items": [
            {
                "id": "evt-plain",
                "status": "confirmed",
                "summary": "Übergabe — 打ち合わせ",
                "iCalUID": "uid-plain@google.com",
                "start": { "dateTime": "2026-07-02T09:00:00+02:00" },
                "end": { "dateTime": "2026-07-02T10:00:00+02:00" },
                "updated": "2026-07-01T00:00:00Z"
            },
            {
                "id": "recur123_20260703T070000Z",
                "status": "confirmed",
                "summary": "Standup",
                "iCalUID": "uid-recur@google.com",
                "start": { "dateTime": "2026-07-03T09:00:00+02:00" },
                "end": { "dateTime": "2026-07-03T09:15:00+02:00" },
                "updated": "2026-07-01T00:00:00Z"
            }
        ],
        "nextPageToken": "page-2"
    }"#;

    /// Page 2 (final): an all-day event; carries the nextSyncToken.
    const FULL_PAGE_2: &str = r#"{
        "kind": "calendar#events",
        "items": [
            {
                "id": "evt-allday",
                "status": "confirmed",
                "summary": "Offsite",
                "iCalUID": "uid-allday@google.com",
                "start": { "date": "2026-07-10" },
                "end": { "date": "2026-07-11" },
                "updated": "2026-07-01T00:00:00Z"
            }
        ],
        "nextSyncToken": "sync-token-1"
    }"#;

    /// Incremental page: one cancelled instance + a moved event.
    const INCREMENTAL_CANCELLED: &str = r#"{
        "kind": "calendar#events",
        "items": [
            { "id": "evt-plain", "status": "cancelled" },
            {
                "id": "recur123_20260703T070000Z",
                "status": "confirmed",
                "summary": "Standup (moved)",
                "iCalUID": "uid-recur@google.com",
                "start": { "dateTime": "2026-07-03T10:00:00+02:00" },
                "end": { "dateTime": "2026-07-03T10:15:00+02:00" },
                "updated": "2026-07-02T00:00:00Z"
            }
        ],
        "nextSyncToken": "sync-token-2"
    }"#;

    /// specs/0054 W5: an incremental page carrying the recurring SERIES MASTER
    /// (it has `recurrence`; expanded instances never do). `singleEvents=true`
    /// only expands on the FULL path, so the incremental page returns the master
    /// whenever the series changes — and the old code cached it as though it were
    /// an occurrence.
    const INCREMENTAL_SERIES_MASTER: &str = r#"{
        "kind": "calendar#events",
        "items": [
            {
                "id": "recur123",
                "status": "confirmed",
                "summary": "Standup (series edited)",
                "iCalUID": "uid-recur@google.com",
                "recurrence": ["RRULE:FREQ=DAILY;COUNT=10"],
                "start": { "dateTime": "2026-07-03T09:00:00+02:00" },
                "end": { "dateTime": "2026-07-03T09:15:00+02:00" },
                "updated": "2026-07-02T00:00:00Z"
            }
        ],
        "nextSyncToken": "sync-token-2"
    }"#;

    /// specs/0054 W5: the whole series cancelled. Arrives keyed by the SERIES id,
    /// which an exact-id delete can never match against `<series>_<instanceTs>`
    /// rows — the "cancelled meeting is stuck on my agenda forever" bug.
    const INCREMENTAL_SERIES_CANCELLED: &str = r#"{
        "kind": "calendar#events",
        "items": [
            {
                "id": "recur123",
                "status": "cancelled",
                "recurrence": ["RRULE:FREQ=DAILY;COUNT=10"]
            }
        ],
        "nextSyncToken": "sync-token-2"
    }"#;

    /// The full-resync page Google returns AFTER the series was cancelled: the
    /// series is simply absent. Used to assert the end state a user sees.
    const FULL_PAGE_AFTER_SERIES_CANCELLED: &str = r#"{
        "kind": "calendar#events",
        "items": [
            {
                "id": "evt-plain",
                "status": "confirmed",
                "summary": "Übergabe — 打ち合わせ",
                "iCalUID": "uid-plain@google.com",
                "start": { "dateTime": "2026-07-02T09:00:00+02:00" },
                "end": { "dateTime": "2026-07-02T10:00:00+02:00" },
                "updated": "2026-07-01T00:00:00Z"
            }
        ],
        "nextSyncToken": "sync-token-3"
    }"#;

    #[test]
    fn redact_email_masks_the_local_part() {
        assert_eq!(redact_email("priya@example.com"), "p***@example.com");
        assert_eq!(redact_email("eng-team@corp.io"), "e***@corp.io");
        // No `@` (defensive) → fully redacted.
        assert_eq!(redact_email("garbage"), "***");
    }

    #[test]
    fn mark_distribution_list_expands_and_clears_by_email() {
        // A DL attendee plus a real person; marking is keyed by normalized email.
        let mut attendees: Vec<serde_json::Value> = vec![
            json!({ "name": "eng-team@x.com", "email": "Eng-Team@X.com", "isDistributionList": true }),
            json!({ "name": "Priya", "email": "priya@x.com", "isDistributionList": false }),
        ];

        // Expanding sets `expanded: true` on the DL (and only the DL); a person
        // is never touched, and re-marking is an idempotent no-op.
        assert!(mark_distribution_list(
            &mut attendees,
            "eng-team@x.com",
            DlMark::Expanded
        ));
        assert_eq!(attendees[0]["expanded"], json!(true));
        assert!(attendees[1].get("expanded").is_none());
        assert!(
            !mark_distribution_list(&mut attendees, "eng-team@x.com", DlMark::Expanded),
            "re-marking an already-expanded DL changes nothing"
        );

        // Clearing the flag demotes the false-positive DL to a person.
        assert!(mark_distribution_list(
            &mut attendees,
            "eng-team@x.com",
            DlMark::ClearFlag
        ));
        assert_eq!(attendees[0]["isDistributionList"], json!(false));

        // A key that matches no DL-flagged attendee is a no-op.
        assert!(!mark_distribution_list(
            &mut attendees,
            "nobody@x.com",
            DlMark::ClearFlag
        ));
    }

    // --- Full sync: pagination + token + horizon ---------------------------

    #[tokio::test]
    async fn full_sync_pages_through_and_stores_token_and_horizon() {
        let pool = memory_db().await;
        let cal = registered_calendar(&pool, "primary").await;
        let script = ScriptedFetch::new(vec![json(FULL_PAGE_1), json(FULL_PAGE_2)]);

        sync_calendar(&pool, &cal, now(), false, &|req| script.call(req))
            .await
            .expect("sync ok");

        // Both pages requested as windowed Full fetches, chained by pageToken.
        let requests = script.requests();
        assert_eq!(requests.len(), 2);
        let (time_min, time_max) = sync_window(now());
        assert_eq!(
            requests[0],
            EventsRequest::Full {
                calendar_id: "primary".into(),
                time_min: time_min.clone(),
                time_max: time_max.clone(),
                page_token: None,
            }
        );
        assert_eq!(
            requests[1],
            EventsRequest::Full {
                calendar_id: "primary".into(),
                time_min,
                time_max: time_max.clone(),
                page_token: Some("page-2".into()),
            }
        );

        // All three events cached (the all-day row exists; the window query
        // hides it later), token + horizon persisted.
        for id in [
            "gcal:primary/evt-plain",
            "gcal:primary/recur123_20260703T070000Z",
            "gcal:primary/evt-allday",
        ] {
            assert!(
                GoogleCalendarRepository::get_event(&pool, id)
                    .await
                    .unwrap()
                    .is_some(),
                "expected cached row {id}"
            );
        }
        let state = GoogleCalendarRepository::get_sync_state(&pool, "primary")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(state.sync_token.as_deref(), Some("sync-token-1"));
        assert_eq!(state.window_ends_at.as_deref(), Some(time_max.as_str()));
        assert!(state.last_synced_at.is_some());
    }

    // --- Incremental: cancellation delete ----------------------------------

    #[tokio::test]
    async fn incremental_sync_deletes_cancelled_instances_and_applies_changes() {
        let pool = memory_db().await;
        // Seed via a full sync, then run an incremental pass on top.
        let cal = registered_calendar(&pool, "primary").await;
        let seed = ScriptedFetch::new(vec![json(FULL_PAGE_1), json(FULL_PAGE_2)]);
        sync_calendar(&pool, &cal, now(), false, &|req| seed.call(req))
            .await
            .unwrap();

        let cal = GoogleCalendarRepository::get_sync_state(&pool, "primary")
            .await
            .unwrap()
            .unwrap();
        let script = ScriptedFetch::new(vec![json(INCREMENTAL_CANCELLED)]);
        sync_calendar(&pool, &cal, now(), false, &|req| script.call(req))
            .await
            .expect("incremental ok");

        // The pass went out as an Incremental fetch with the stored token.
        assert_eq!(
            script.requests(),
            vec![EventsRequest::Incremental {
                calendar_id: "primary".into(),
                sync_token: "sync-token-1".into(),
                page_token: None,
            }]
        );
        // Cancelled instance gone; the moved instance was rewritten in place.
        assert!(
            GoogleCalendarRepository::get_event(&pool, "gcal:primary/evt-plain")
                .await
                .unwrap()
                .is_none()
        );
        let moved =
            GoogleCalendarRepository::get_event(&pool, "gcal:primary/recur123_20260703T070000Z")
                .await
                .unwrap()
                .expect("moved row");
        assert_eq!(moved.title, "Standup (moved)");
        assert_eq!(moved.starts_at, "2026-07-03T08:00:00+00:00");
        // New token stored.
        let state = GoogleCalendarRepository::get_sync_state(&pool, "primary")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(state.sync_token.as_deref(), Some("sync-token-2"));
    }

    // --- specs/0054 W5: recurring series masters -----------------------------

    /// A series master must never become a cache row. Caching it creates a
    /// phantom meeting at the series' first-occurrence time that no cancellation
    /// can ever delete (cancellations are keyed `<series>_<instanceTs>`), which is
    /// how the reporter accumulated 384 undeletable rows.
    #[tokio::test]
    async fn incremental_sync_never_caches_a_recurring_master() {
        let pool = memory_db().await;
        let cal = registered_calendar(&pool, "primary").await;
        let seed = ScriptedFetch::new(vec![json(FULL_PAGE_1), json(FULL_PAGE_2)]);
        sync_calendar(&pool, &cal, now(), false, &|req| seed.call(req))
            .await
            .unwrap();

        let cal = GoogleCalendarRepository::get_sync_state(&pool, "primary")
            .await
            .unwrap()
            .unwrap();
        // The master triggers a full resync, so the follow-up Full pages are scripted too.
        let script = ScriptedFetch::new(vec![
            json(INCREMENTAL_SERIES_MASTER),
            json(FULL_PAGE_1),
            json(FULL_PAGE_2),
        ]);
        sync_calendar(&pool, &cal, now(), false, &|req| script.call(req))
            .await
            .expect("incremental ok");

        assert!(
            GoogleCalendarRepository::get_event(&pool, "gcal:primary/recur123")
                .await
                .unwrap()
                .is_none(),
            "the series MASTER must never be cached as an occurrence"
        );
        // The incremental pass hands off to a FULL sync, which is the only path
        // that expands `singleEvents=true` — so the occurrence comes back properly.
        assert!(
            GoogleCalendarRepository::get_event(&pool, "gcal:primary/recur123_20260703T070000Z")
                .await
                .unwrap()
                .is_some(),
            "the full resync must re-expand the series into real occurrences"
        );
        assert!(
            matches!(script.requests().last(), Some(EventsRequest::Full { .. })),
            "a series master must escalate to a full resync, got {:?}",
            script.requests()
        );
    }

    /// A cancelled series removes EVERY cached row for it, not just an exact-id
    /// match (which matches nothing, since instances are `<series>_<ts>`).
    #[tokio::test]
    async fn cancelled_series_master_removes_the_whole_series() {
        let pool = memory_db().await;
        let cal = registered_calendar(&pool, "primary").await;
        let seed = ScriptedFetch::new(vec![json(FULL_PAGE_1), json(FULL_PAGE_2)]);
        sync_calendar(&pool, &cal, now(), false, &|req| seed.call(req))
            .await
            .unwrap();

        let cal = GoogleCalendarRepository::get_sync_state(&pool, "primary")
            .await
            .unwrap()
            .unwrap();
        // After a cancellation Google's full resync simply omits the series.
        let script = ScriptedFetch::new(vec![
            json(INCREMENTAL_SERIES_CANCELLED),
            json(FULL_PAGE_AFTER_SERIES_CANCELLED),
        ]);
        sync_calendar(&pool, &cal, now(), false, &|req| script.call(req))
            .await
            .expect("incremental ok");

        assert!(
            GoogleCalendarRepository::get_event(&pool, "gcal:primary/recur123_20260703T070000Z")
                .await
                .unwrap()
                .is_none(),
            "cancelling the series must remove its expanded occurrences"
        );
        assert!(
            GoogleCalendarRepository::get_event(&pool, "gcal:primary/recur123")
                .await
                .unwrap()
                .is_none(),
            "and must leave no series-level row behind either"
        );
        // An unrelated single event is untouched.
        assert!(
            GoogleCalendarRepository::get_event(&pool, "gcal:primary/evt-plain")
                .await
                .unwrap()
                .is_some(),
            "a series cancellation must not touch unrelated events"
        );
    }

    /// Regression guard for the specs/0032 behaviour W5 modifies: a cancelled
    /// single INSTANCE still removes exactly its own row.
    #[tokio::test]
    async fn cancelled_instance_still_removes_only_itself() {
        let pool = memory_db().await;
        let cal = registered_calendar(&pool, "primary").await;
        let seed = ScriptedFetch::new(vec![json(FULL_PAGE_1), json(FULL_PAGE_2)]);
        sync_calendar(&pool, &cal, now(), false, &|req| seed.call(req))
            .await
            .unwrap();

        let cal = GoogleCalendarRepository::get_sync_state(&pool, "primary")
            .await
            .unwrap()
            .unwrap();
        let script = ScriptedFetch::new(vec![json(INCREMENTAL_CANCELLED)]);
        sync_calendar(&pool, &cal, now(), false, &|req| script.call(req))
            .await
            .expect("incremental ok");

        assert!(
            GoogleCalendarRepository::get_event(&pool, "gcal:primary/evt-plain")
                .await
                .unwrap()
                .is_none(),
            "the cancelled instance is removed"
        );
        assert!(
            GoogleCalendarRepository::get_event(&pool, "gcal:primary/recur123_20260703T070000Z")
                .await
                .unwrap()
                .is_some(),
            "a sibling occurrence must survive an instance cancellation"
        );
    }

    // --- 410 GONE → transparent full resync ---------------------------------

    #[tokio::test]
    async fn expired_sync_token_falls_back_to_full_resync() {
        let pool = memory_db().await;
        let cal = registered_calendar(&pool, "primary").await;
        let seed = ScriptedFetch::new(vec![json(FULL_PAGE_1), json(FULL_PAGE_2)]);
        sync_calendar(&pool, &cal, now(), false, &|req| seed.call(req))
            .await
            .unwrap();

        // A stale row that the full resync's replace must prune away.
        let mut stale = GoogleCalendarRepository::get_event(&pool, "gcal:primary/evt-plain")
            .await
            .unwrap()
            .unwrap();
        stale.id = "gcal:primary/evt-out-of-window".into();
        GoogleCalendarRepository::upsert_event(&pool, &stale)
            .await
            .unwrap();

        let cal = GoogleCalendarRepository::get_sync_state(&pool, "primary")
            .await
            .unwrap()
            .unwrap();
        // Incremental hits 410, then the full resync serves one final page.
        let script = ScriptedFetch::new(vec![Ok(FetchBody::Gone), json(FULL_PAGE_2)]);
        sync_calendar(&pool, &cal, now(), false, &|req| script.call(req))
            .await
            .expect("recovers via full resync");

        let requests = script.requests();
        assert_eq!(requests.len(), 2);
        assert!(matches!(requests[0], EventsRequest::Incremental { .. }));
        assert!(matches!(requests[1], EventsRequest::Full { .. }));

        // The replace pruned every pre-410 row; only the resync page remains.
        for gone in [
            "gcal:primary/evt-plain",
            "gcal:primary/evt-out-of-window",
            "gcal:primary/recur123_20260703T070000Z",
        ] {
            assert!(
                GoogleCalendarRepository::get_event(&pool, gone)
                    .await
                    .unwrap()
                    .is_none(),
                "expected {gone} pruned by the full resync"
            );
        }
        assert!(
            GoogleCalendarRepository::get_event(&pool, "gcal:primary/evt-allday")
                .await
                .unwrap()
                .is_some()
        );
        let state = GoogleCalendarRepository::get_sync_state(&pool, "primary")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(state.sync_token.as_deref(), Some("sync-token-1"));
    }

    // --- Horizon re-extension ------------------------------------------------

    #[tokio::test]
    async fn near_horizon_triggers_full_resync_despite_valid_token() {
        let pool = memory_db().await;
        let cal = registered_calendar(&pool, "primary").await;
        let seed = ScriptedFetch::new(vec![json(FULL_PAGE_1), json(FULL_PAGE_2)]);
        let t0 = now();
        sync_calendar(&pool, &cal, t0, false, &|req| seed.call(req))
            .await
            .unwrap();

        // 55 days later: within 7 days of the 60-day horizon → full resync.
        let later = t0 + Duration::days(55);
        let cal = GoogleCalendarRepository::get_sync_state(&pool, "primary")
            .await
            .unwrap()
            .unwrap();
        assert!(cal.sync_token.is_some(), "token exists but horizon is near");
        let script = ScriptedFetch::new(vec![json(FULL_PAGE_2)]);
        sync_calendar(&pool, &cal, later, false, &|req| script.call(req))
            .await
            .expect("re-extends via full resync");

        let requests = script.requests();
        assert_eq!(requests.len(), 1);
        match &requests[0] {
            EventsRequest::Full { time_max, .. } => {
                assert_eq!(
                    time_max,
                    &(later + Duration::days(WINDOW_HORIZON_DAYS)).to_rfc3339(),
                    "the window must be re-extended from the new now"
                );
            }
            other => panic!("expected a Full request, got {other:?}"),
        }
        // Rows outside the new window were pruned by the replace.
        assert!(
            GoogleCalendarRepository::get_event(&pool, "gcal:primary/evt-plain")
                .await
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn horizon_health_boundary() {
        let t = now();
        let healthy = (t + Duration::days(HORIZON_SLACK_DAYS + 1)).to_rfc3339();
        let near = (t + Duration::days(HORIZON_SLACK_DAYS - 1)).to_rfc3339();
        assert!(horizon_healthy(Some(&healthy), t));
        assert!(!horizon_healthy(Some(&near), t));
        assert!(
            !horizon_healthy(None, t),
            "unknown horizon forces full sync"
        );
        assert!(!horizon_healthy(Some("garbage"), t));
    }

    /// specs/0054 W5: the horizon check alone allows roughly 53 days of unbroken
    /// incremental syncs (60-day window, 7-day slack), so any cache drift persists
    /// for weeks — the reporter's last full sync was 15 days before the bug report.
    /// Cap the incremental streak independently of the horizon.
    #[test]
    fn a_stale_last_sync_forces_a_full_resync_even_with_a_healthy_horizon() {
        let t = now();
        let far_horizon = (t + Duration::days(HORIZON_SLACK_DAYS + 30)).to_rfc3339();
        let recent = (t - Duration::days(1)).to_rfc3339();
        let stale = (t - Duration::days(MAX_INCREMENTAL_DAYS + 1)).to_rfc3339();

        assert!(
            incremental_is_safe(Some(&far_horizon), Some(&recent), t),
            "a healthy horizon plus a recent sync stays incremental"
        );
        assert!(
            !incremental_is_safe(Some(&far_horizon), Some(&stale), t),
            "a stale last sync must force a full resync despite a healthy horizon"
        );
        assert!(
            !incremental_is_safe(Some(&far_horizon), None, t),
            "an unknown last-sync time must force a full resync"
        );
        assert!(
            !incremental_is_safe(Some(&far_horizon), Some("garbage"), t),
            "an unparseable last-sync time must force a full resync"
        );
        assert!(
            !incremental_is_safe(None, Some(&recent), t),
            "the horizon check still applies"
        );
    }

    /// The predicate above is only useful if `sync_calendar` actually consults it.
    /// Seed a calendar with a valid token and a far horizon but a LAST SYNC older
    /// than the cap, and assert the pass goes out as a Full request.
    #[tokio::test]
    async fn a_stale_calendar_resyncs_fully_at_the_call_site() {
        let pool = memory_db().await;
        registered_calendar(&pool, "primary").await;
        let t = now();
        sqlx::query(
            "UPDATE google_calendar_sync
                SET sync_token = 'tok', window_ends_at = ?, last_synced_at = ?
              WHERE calendar_id = 'primary'",
        )
        .bind((t + Duration::days(HORIZON_SLACK_DAYS + 30)).to_rfc3339())
        .bind((t - Duration::days(MAX_INCREMENTAL_DAYS + 1)).to_rfc3339())
        .execute(&pool)
        .await
        .unwrap();

        let cal = GoogleCalendarRepository::get_sync_state(&pool, "primary")
            .await
            .unwrap()
            .unwrap();
        let script = ScriptedFetch::new(vec![json(FULL_PAGE_1), json(FULL_PAGE_2)]);
        sync_calendar(&pool, &cal, t, false, &|req| script.call(req))
            .await
            .expect("sync ok");

        assert!(
            script
                .requests()
                .iter()
                .all(|r| matches!(r, EventsRequest::Full { .. })),
            "a stale calendar must resync fully, got {:?}",
            script.requests()
        );
    }

    // --- Cache readers -------------------------------------------------------

    // Source-SWITCH continuity (spec 0032 acceptance criterion 5): a
    // Google-sourced item must produce the exact `ext:<uid>@<start>` dismissal
    // key its EventKit twin produces, so an event dismissed while EventKit was
    // the active source stays dismissed after the user connects Google (and
    // vice versa after a disconnect).
    #[test]
    fn google_item_produces_the_same_dismiss_key_as_its_eventkit_twin() {
        let start = Utc.with_ymd_and_hms(2026, 7, 2, 9, 0, 0).unwrap();
        // The Google side stores chrono to_rfc3339 UTC; the EventKit twin of
        // the same instant arrives through eventkit.rs's identical formatting.
        let row = GoogleCalendarEventRow {
            id: "gcal:primary/g1_20260702T090000Z".into(),
            calendar_id: "primary".into(),
            ical_uid: Some("uid-1@google.com".into()),
            title: "Standup".into(),
            starts_at: start.to_rfc3339(),
            ends_at: (start + Duration::hours(1)).to_rfc3339(),
            is_all_day: false,
            location: None,
            zoom_url: None,
            organizer_email: None,
            my_response: Some("accepted".into()),
            attendees_json: "[]".into(),
            status: "confirmed".into(),
            updated_at: start.to_rfc3339(),
        };
        let google_item = to_upcoming_meeting(&row, None);
        let eventkit_twin = UpcomingMeeting {
            id: "ek-1".into(),
            title: "Standup".into(),
            starts_at: start.to_rfc3339(),
            ends_at: (start + Duration::hours(1)).to_rfc3339(),
            calendar_name: "Work".into(),
            location: None,
            zoom_url: None,
            external_id: Some("uid-1@google.com".into()),
        };

        let google_key = crate::calendar::day_agenda::stable_dismiss_key(&google_item)
            .expect("google item has an external id");
        let eventkit_key = crate::calendar::day_agenda::stable_dismiss_key(&eventkit_twin)
            .expect("eventkit twin has an external id");

        assert_eq!(google_key, eventkit_key, "dismissal keys must transfer");
        assert_eq!(
            google_key,
            format!("ext:uid-1@google.com@{}", start.to_rfc3339())
        );
    }

    #[tokio::test]
    async fn cached_attendees_by_title_time_matches_closest_title() {
        let pool = memory_db().await;
        registered_calendar(&pool, "primary").await;
        let event = parse_fixture_event(ATTENDEE_EVENT);
        let row = map_event("primary", &event, false).unwrap();
        GoogleCalendarRepository::upsert_event(&pool, &row)
            .await
            .unwrap();

        // Recording started 20 minutes after the event; title case differs.
        let attendees =
            cached_attendees_by_title_time(&pool, "roadmap review", "2026-07-02T15:20:00+00:00")
                .await;
        assert_eq!(attendees.len(), 4);
        assert!(attendees.iter().any(|a| a.is_current_user));

        // A non-matching title finds nothing.
        let none =
            cached_attendees_by_title_time(&pool, "different", "2026-07-02T15:20:00+00:00").await;
        assert!(none.is_empty());
    }

    // --- specs/0041 WS5: iCalUID dedup at the cached read path ----------------

    fn dup_row(
        calendar_id: &str,
        event_id: &str,
        ical_uid: Option<&str>,
        starts_at: &str,
        organizer_email: Option<&str>,
    ) -> GoogleCalendarEventRow {
        GoogleCalendarEventRow {
            id: format!("gcal:{calendar_id}/{event_id}"),
            calendar_id: calendar_id.into(),
            ical_uid: ical_uid.map(str::to_string),
            title: "Standup".into(),
            starts_at: starts_at.into(),
            ends_at: "2026-07-08T10:00:00+00:00".into(),
            is_all_day: false,
            location: None,
            zoom_url: None,
            organizer_email: organizer_email.map(str::to_string),
            my_response: Some("accepted".into()),
            attendees_json: "[]".into(),
            status: "confirmed".into(),
            updated_at: "2026-07-08T00:00:00+00:00".into(),
        }
    }

    #[test]
    fn dedup_collapses_same_ical_uid_preferring_the_organizers_calendar() {
        let t = "2026-07-08T09:00:00+00:00";
        // Same invite cached from two synced calendars; the organizer's own
        // calendar ("boss@x.com") is listed SECOND and must still win.
        let rows = vec![
            dup_row(
                "team@group.calendar.google.com",
                "evt1",
                Some("uid-1"),
                t,
                Some("boss@x.com"),
            ),
            dup_row("boss@x.com", "evt1", Some("uid-1"), t, Some("boss@x.com")),
            dup_row("boss@x.com", "evt2", Some("uid-2"), t, Some("boss@x.com")),
        ];
        let out = dedup_rows_by_ical_uid(rows, Some("me@x.com"));
        assert_eq!(
            out.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
            vec!["gcal:boss@x.com/evt1", "gcal:boss@x.com/evt2"],
            "one bubble per iCalUID; the organizer's calendar row survives"
        );
    }

    #[test]
    fn dedup_falls_back_to_the_primary_calendar_then_first() {
        let t = "2026-07-08T09:00:00+00:00";
        // No row belongs to the organizer's calendar → the primary (account
        // email) wins; with neither, the first row encountered survives.
        let rows = vec![
            dup_row(
                "team@group.calendar.google.com",
                "evt1",
                Some("uid-1"),
                t,
                Some("ext@y.com"),
            ),
            dup_row("me@x.com", "evt1", Some("uid-1"), t, Some("ext@y.com")),
            dup_row("cal-a", "evt3", Some("uid-3"), t, None),
            dup_row("cal-b", "evt3", Some("uid-3"), t, None),
        ];
        let out = dedup_rows_by_ical_uid(rows, Some("me@x.com"));
        assert_eq!(
            out.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
            vec!["gcal:me@x.com/evt1", "gcal:cal-a/evt3"],
            "primary calendar preferred; else the first copy stands"
        );
    }

    #[test]
    fn dedup_keeps_recurring_occurrences_and_uidless_rows() {
        let rows = vec![
            // Same UID, different occurrence starts (a recurring series) — both stay.
            dup_row(
                "cal-a",
                "r_1",
                Some("uid-r"),
                "2026-07-08T09:00:00+00:00",
                None,
            ),
            dup_row(
                "cal-a",
                "r_2",
                Some("uid-r"),
                "2026-07-08T14:00:00+00:00",
                None,
            ),
            // No UID — never collapsed, even with identical starts.
            dup_row("cal-a", "n1", None, "2026-07-08T09:00:00+00:00", None),
            dup_row("cal-b", "n2", None, "2026-07-08T09:00:00+00:00", None),
        ];
        assert_eq!(dedup_rows_by_ical_uid(rows, None).len(), 4);
    }

    /// The collapse must not break dismissals (specs/0041 WS5): every duplicate
    /// shares the iCalUID + start, so the `ext:<uid>@<start>` dismiss key is
    /// identical no matter which row survives.
    #[test]
    fn dedup_survivor_keeps_the_same_dismiss_key_as_the_dropped_duplicate() {
        let t = "2026-07-08T09:00:00+00:00";
        let a = dup_row("cal-a", "evt1", Some("uid-1"), t, None);
        let b = dup_row("me@x.com", "evt1", Some("uid-1"), t, None);
        let key_of = |row: &GoogleCalendarEventRow| {
            crate::calendar::day_agenda::stable_dismiss_key(&to_upcoming_meeting(row, None))
                .expect("row has an iCalUID")
        };
        let (key_a, key_b) = (key_of(&a), key_of(&b));
        assert_eq!(key_a, key_b, "duplicates share one dismiss key");

        let out = dedup_rows_by_ical_uid(vec![a, b], Some("me@x.com"));
        assert_eq!(out.len(), 1);
        assert_eq!(
            key_of(&out[0]),
            key_a,
            "a dismissal stored before the collapse still matches the survivor"
        );
    }

    // --- specs/0041 WS5: selection defaults on calendar-list registration -----

    fn list_item(id: &str, primary: bool) -> CalendarListItem {
        CalendarListItem {
            id: id.into(),
            summary: Some(format!("Calendar {id}")),
            summary_override: None,
            primary,
        }
    }

    async fn selected_map(pool: &SqlitePool) -> std::collections::HashMap<String, bool> {
        GoogleCalendarRepository::list_sync_states(pool)
            .await
            .unwrap()
            .into_iter()
            .map(|s| (s.calendar_id, s.selected))
            .collect()
    }

    #[tokio::test]
    async fn more_than_five_calendars_default_to_primary_only() {
        let pool = memory_db().await;
        let mut items: Vec<CalendarListItem> = (1..=5)
            .map(|i| list_item(&format!("cal-{i}"), false))
            .collect();
        items.push(list_item("me@x.com", true));

        let primary = register_calendar_list(&pool, &items).await.unwrap();
        assert_eq!(primary.as_deref(), Some("me@x.com"));

        let selected = selected_map(&pool).await;
        assert_eq!(selected.len(), 6);
        assert!(selected["me@x.com"], "the primary calendar stays selected");
        for i in 1..=5 {
            assert!(
                !selected[&format!("cal-{i}")],
                "a >5-calendar account defaults non-primary calendars off"
            );
        }
    }

    #[tokio::test]
    async fn five_or_fewer_calendars_keep_the_all_selected_default() {
        let pool = memory_db().await;
        let mut items: Vec<CalendarListItem> = (1..=4)
            .map(|i| list_item(&format!("cal-{i}"), false))
            .collect();
        items.push(list_item("me@x.com", true));

        register_calendar_list(&pool, &items).await.unwrap();

        let selected = selected_map(&pool).await;
        assert_eq!(selected.len(), 5);
        assert!(
            selected.values().all(|&s| s),
            "small accounts keep today's all-selected behavior"
        );
    }

    #[tokio::test]
    async fn registration_never_clobbers_an_existing_selection() {
        let pool = memory_db().await;
        // The user already chose: cal-1 OFF, cal-2 ON.
        GoogleCalendarRepository::upsert_sync_state(&pool, "cal-1", "One", true)
            .await
            .unwrap();
        GoogleCalendarRepository::set_calendar_selected(&pool, "cal-1", false)
            .await
            .unwrap();
        GoogleCalendarRepository::upsert_sync_state(&pool, "cal-2", "Two", false)
            .await
            .unwrap();
        GoogleCalendarRepository::set_calendar_selected(&pool, "cal-2", true)
            .await
            .unwrap();

        // A big-account re-registration (default OFF) + a small-account one
        // (default ON) — neither touches the user's values.
        let mut items: Vec<CalendarListItem> = (1..=7)
            .map(|i| list_item(&format!("cal-{i}"), false))
            .collect();
        items.push(list_item("me@x.com", true));
        register_calendar_list(&pool, &items).await.unwrap();
        let selected = selected_map(&pool).await;
        assert!(!selected["cal-1"], "user's OFF preserved");
        assert!(selected["cal-2"], "user's ON preserved");
        assert!(!selected["cal-3"], "NEW rows get the >5 default (off)");
        assert!(selected["me@x.com"]);

        register_calendar_list(
            &pool,
            &[list_item("cal-1", false), list_item("cal-2", false)],
        )
        .await
        .unwrap();
        let selected = selected_map(&pool).await;
        assert!(!selected["cal-1"], "small-account re-sync still keeps OFF");
        assert!(selected["cal-2"]);
    }
}
