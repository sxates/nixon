//! Tauri IPC for the Google Calendar provider (specs/0032 task 5).
//!
//! The five `api_google_calendar_*` commands the Settings card drives, plus
//! the `google-calendar-auth-required` event (emitted from the sync path, see
//! `sync::AUTH_REQUIRED_EVENT`). The wire shapes here are the **pinned
//! contract** `frontend/src/lib/googleCalendar.ts` was built against — do not
//! change field names or casing without changing both sides.
//!
//! House pattern: internals are `anyhow::Result`; the command boundary maps to
//! user-displayable `String`s (the frontend shows connect errors verbatim).
//! Token material never appears in errors or logs (ADR-0010).

use anyhow::{anyhow, bail, Result};
use serde::Serialize;
use tauri::{AppHandle, Runtime};

use super::{capabilities, oauth, sync};
use crate::database::repositories::attendee_photos::AttendeePhotosRepository;
use crate::database::repositories::google_calendar::GoogleCalendarRepository;
use crate::database::repositories::owner_emails::OwnerEmailsRepository;
use crate::secrets;

// ---------------------------------------------------------------------------
// Wire shapes (pinned — mirrored by lib/googleCalendar.ts)
// ---------------------------------------------------------------------------

/// `api_google_calendar_status` result: `{ configured, connected, email,
/// lastSyncedAt, calendars: [{id, summary, selected}] }`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GoogleCalendarStatusDto {
    /// An OAuth client id was baked into this build (`NIXON_GOOGLE_CLIENT_ID`).
    pub configured: bool,
    /// An account row exists (connect completed and wasn't disconnected).
    pub connected: bool,
    pub email: Option<String>,
    /// Newest `last_synced_at` across the account's calendars (RFC3339 UTC).
    pub last_synced_at: Option<String>,
    pub calendars: Vec<GoogleCalendarEntryDto>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GoogleCalendarEntryDto {
    pub id: String,
    pub summary: String,
    pub selected: bool,
}

/// `api_google_calendar_connect` result: `{ email }`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GoogleCalendarConnectDto {
    pub email: String,
}

const DB_NOT_READY: &str = "Nixon's database isn't ready yet. Please try again in a moment.";

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

/// Connection status for the Settings card. `connected` is driven purely by
/// the local account row — no network, so this stays instant and offline-safe.
#[tauri::command]
pub async fn api_google_calendar_status<R: Runtime>(
    app: AppHandle<R>,
) -> Result<GoogleCalendarStatusDto, String> {
    let Some(pool) = sync::db_pool(&app) else {
        return Err(DB_NOT_READY.to_string());
    };
    let account = GoogleCalendarRepository::get_account(&pool)
        .await
        .map_err(|e| format!("Could not read the Google Calendar connection state: {e}"))?;
    let states = GoogleCalendarRepository::list_sync_states(&pool)
        .await
        .map_err(|e| format!("Could not read the Google Calendar list: {e}"))?;

    // RFC3339 UTC strings compare chronologically, so lexicographic max works.
    let last_synced_at = states.iter().filter_map(|s| s.last_synced_at.clone()).max();

    Ok(GoogleCalendarStatusDto {
        configured: super::is_configured(),
        connected: account.is_some(),
        email: account.map(|a| a.email),
        last_synced_at,
        calendars: states
            .into_iter()
            .map(|s| GoogleCalendarEntryDto {
                id: s.calendar_id,
                summary: s.summary,
                selected: s.selected,
            })
            .collect(),
    })
}

/// Run the full connect flow: loopback listener + browser consent (via the
/// allowlisted external-URL opener), PKCE exchange, Keychain store, calendar
/// list fetch (account identity), initial sync. Long-running — the OAuth layer
/// times out after 5 minutes if the user abandons the consent screen.
///
/// Every failure leg returns a user-friendly string (shown verbatim by the
/// frontend); a Keychain failure aborts with nothing persisted (ADR-0010) and
/// any later failure cleans the just-stored token back out, so a failed
/// connect never leaves a half-connected state.
#[tauri::command]
pub async fn api_google_calendar_connect<R: Runtime>(
    app: AppHandle<R>,
) -> Result<GoogleCalendarConnectDto, String> {
    match connect_flow(&app).await {
        Ok(email) => Ok(GoogleCalendarConnectDto { email }),
        Err(e) => {
            log::warn!("google calendar: connect failed: {e:#}");
            Err(e.to_string()) // top-level context is the user-facing message
        }
    }
}

async fn connect_flow<R: Runtime>(app: &AppHandle<R>) -> Result<String> {
    if !super::is_configured() {
        bail!("Google Calendar is not configured in this build of Nixon.");
    }
    let Some(pool) = sync::db_pool(app) else {
        bail!("{DB_NOT_READY}");
    };
    // ADR-0010: Keychain or nothing — without a store the connect cannot start.
    let Some(store) = secrets::store() else {
        bail!(
            "The macOS Keychain isn't available, so the Google connection can't be \
             stored securely. Nothing was saved."
        );
    };

    let pending = oauth::start_connect_flow().await?;
    // Browser open goes through the app's allowlisted opener (https only ever
    // reaches the OS `open`); the flow keeps waiting on the loopback listener.
    crate::utils::open_external_url(pending.auth_url.clone())
        .await
        .map_err(|e| anyhow!("Could not open the browser for Google sign-in: {e}"))?;

    // Waits for the redirect (5-min abandonment timeout → a clean "cancelled"
    // message), exchanges the code, and persists the refresh token — or fails
    // with nothing persisted anywhere.
    let bundle = pending.finish(store).await?;

    match finish_connect(app, &pool, &bundle.access_token).await {
        Ok(email) => Ok(email),
        Err(e) => {
            // The refresh token was already in the Keychain; a half-connected
            // state (token without account identity) must not survive.
            if let Err(cleanup) = GoogleCalendarRepository::purge_all(&pool).await {
                log::warn!("google calendar: post-failure purge also failed: {cleanup}");
            }
            if let Err(cleanup) = oauth::revoke_and_clear(store).await {
                log::warn!("google calendar: post-failure token cleanup also failed: {cleanup:#}");
            }
            Err(e)
        }
    }
}

/// The post-token half of connect: account identity + initial sync.
async fn finish_connect<R: Runtime>(
    app: &AppHandle<R>,
    pool: &sqlx::SqlitePool,
    access_token: &str,
) -> Result<String> {
    let client = sync::http_client()?;
    let email = sync::sync_calendar_list(&client, access_token, pool)
        .await
        .map_err(|e| anyhow!("Connected to Google, but the calendar list couldn't be loaded: {e}"))?
        .ok_or_else(|| {
            anyhow!("Connected to Google, but no primary calendar was found on the account.")
        })?;
    GoogleCalendarRepository::set_account(pool, &email)
        .await
        .map_err(|e| anyhow!("Could not save the Google Calendar connection: {e}"))?;
    sync::clear_auth_required();

    // specs/0018 + 0032: the connected Google account's email is authoritative
    // "this is me" evidence, so auto-declare it as an owner email for participant
    // seeding. `OwnerEmailsRepository::add` is idempotent (INSERT OR IGNORE on
    // the normalized address), so a reconnect is a silent no-op. Best-effort —
    // a failure here must not fail the connect. Deliberately NOT undone on
    // disconnect: the address is still the user's; owner emails stay
    // user-managed in Settings.
    if let Err(e) = OwnerEmailsRepository::add(pool, &email).await {
        log::warn!(
            "google calendar: could not record the connected account as an owner email: {e}"
        );
    }

    // Initial sync (spec 0032): populate the cache right away. Best-effort —
    // a transient failure must not undo a successful connect; the staleness
    // triggers and the 10-minute timer will fill the cache shortly.
    if let Err(e) = sync::sync_all(app).await {
        log::warn!("google calendar: initial sync after connect failed (will retry): {e:#}");
    }

    // Best-effort capability probe (specs/0038 WS3): a granted scope is NOT
    // access, so probe once what the org actually allows (DL expansion / photos)
    // and cache the flags. Spawned so it never delays the connect return, and
    // fully non-fatal — a probe failure can't undo a successful connect. Runs
    // after the initial sync so it can probe on a real DL attendee if present.
    let probe_app = app.clone();
    tauri::async_runtime::spawn(async move {
        capabilities::probe_and_store(&probe_app).await;
    });

    Ok(email)
}

/// Disconnect: best-effort token revoke at Google + Keychain delete, then
/// purge all `google_calendar_*` tables. `dismissed_calendar_events` is
/// deliberately untouched — dismissal keys are cross-source (0029 WS6.2).
#[tauri::command]
pub async fn api_google_calendar_disconnect<R: Runtime>(app: AppHandle<R>) -> Result<(), String> {
    let Some(pool) = sync::db_pool(&app) else {
        return Err(DB_NOT_READY.to_string());
    };
    // Revoke + Keychain delete first (best-effort revoke inside; a failed
    // Keychain DELETE is surfaced below — the user should know a token is
    // stuck — but never blocks the local purge).
    let keychain_result = match secrets::store() {
        Some(store) => oauth::revoke_and_clear(store).await,
        None => Ok(()), // no store ⇒ no token was ever persisted
    };
    GoogleCalendarRepository::purge_all(&pool)
        .await
        .map_err(|e| format!("Could not clear the local Google Calendar data: {e}"))?;
    // Purge cached attendee photos too (specs/0038 WS3, ADR-0010): no photo data
    // may survive a disconnect. Best-effort — a failure here is logged but must
    // not block the disconnect (the calendar tables are already cleared).
    if let Err(e) = AttendeePhotosRepository::clear_photos(&pool).await {
        log::warn!("google calendar: could not clear cached attendee photos on disconnect: {e}");
    }
    sync::clear_auth_required();
    keychain_result.map_err(|e| e.to_string())
}

/// One calendar's selection mutation, shared by the single and bulk commands: flip the
/// flag, invalidate the incremental sync token (either direction — ON needs a full
/// resync to restore the events deleted at deselect time, an incremental pass would
/// only replay changes), and on deselect drop the cached events now so the agenda
/// updates immediately. The caller decides where the (expensive, single-flight)
/// `sync_all` goes: after the one calendar, or once after a whole batch.
async fn apply_calendar_selection(
    pool: &sqlx::SqlitePool,
    calendar_id: &str,
    selected: bool,
) -> Result<(), String> {
    GoogleCalendarRepository::set_calendar_selected(pool, calendar_id, selected)
        .await
        .map_err(|e| format!("Could not update the calendar selection: {e}"))?;
    GoogleCalendarRepository::set_sync_progress(pool, calendar_id, None)
        .await
        .map_err(|e| format!("Could not reset the calendar's sync state: {e}"))?;
    if !selected {
        GoogleCalendarRepository::delete_events_for_calendar(pool, calendar_id)
            .await
            .map_err(|e| format!("Could not remove the calendar's cached events: {e}"))?;
    }
    Ok(())
}

/// Toggle one calendar's sync selection (see [`apply_calendar_selection`] for the
/// token-invalidation semantics).
#[tauri::command]
pub async fn api_google_calendar_set_calendar_selected<R: Runtime>(
    app: AppHandle<R>,
    calendar_id: String,
    selected: bool,
) -> Result<(), String> {
    let Some(pool) = sync::db_pool(&app) else {
        return Err(DB_NOT_READY.to_string());
    };
    apply_calendar_selection(&pool, &calendar_id, selected).await?;
    if selected {
        sync::sync_all(&app)
            .await
            .map_err(|e| format!("The calendar was enabled, but its first sync failed: {e}"))?;
    }
    Ok(())
}

/// Bulk form of [`api_google_calendar_set_calendar_selected`] for the Settings
/// "Select all / none" actions (specs/0041 WS5). One command instead of N
/// client-batched invokes because each per-calendar enable runs a FULL
/// `sync_all` pass: 30 serial invokes would sync quadratically, and parallel
/// invokes race the single-flight guard (whichever grabs the lock first syncs a
/// partial selection and the rest no-op, leaving newly-enabled calendars empty
/// until the next staleness pass). Here every selection lands first, then ONE
/// sync covers them all; deselects just drop their cached events.
#[tauri::command]
pub async fn api_google_calendar_set_calendars_selected<R: Runtime>(
    app: AppHandle<R>,
    calendar_ids: Vec<String>,
    selected: bool,
) -> Result<(), String> {
    let Some(pool) = sync::db_pool(&app) else {
        return Err(DB_NOT_READY.to_string());
    };
    for calendar_id in &calendar_ids {
        apply_calendar_selection(&pool, calendar_id, selected).await?;
    }
    if selected && !calendar_ids.is_empty() {
        sync::sync_all(&app)
            .await
            .map_err(|e| format!("The calendars were enabled, but their first sync failed: {e}"))?;
    }
    Ok(())
}

/// Manual "Sync now" — forces a pass regardless of staleness (still coalesced
/// by the single-flight guard if one is already running).
#[tauri::command]
pub async fn api_google_calendar_sync_now<R: Runtime>(app: AppHandle<R>) -> Result<(), String> {
    sync::sync_all(&app)
        .await
        .map_err(|e| format!("Google Calendar sync failed: {e}"))
}

/// `api_google_capabilities` result: the probed best-effort enrichment flags for
/// the Settings surface (specs/0038 WS3). Each is `null` until probed, and a
/// granted scope is NOT access — `false` means the org denied it at runtime.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GoogleCapabilitiesDto {
    pub can_expand_groups: Option<bool>,
    pub can_fetch_photos: Option<bool>,
    /// RFC3339 UTC of the last probe (`null` = never probed).
    pub probed_at: Option<String>,
}

/// The connected account's probed capabilities (specs/0038 WS3). Instant/local —
/// no network. Returns all-`null` when not connected or not yet probed, so the
/// Settings UI can render "checking…" / "not available" without special-casing.
#[tauri::command]
pub async fn api_google_capabilities<R: Runtime>(
    app: AppHandle<R>,
) -> Result<GoogleCapabilitiesDto, String> {
    let Some(pool) = sync::db_pool(&app) else {
        return Err(DB_NOT_READY.to_string());
    };
    let caps = GoogleCalendarRepository::get_capabilities(&pool)
        .await
        .map_err(|e| format!("Could not read the Google account capabilities: {e}"))?;
    Ok(match caps {
        Some(c) => GoogleCapabilitiesDto {
            can_expand_groups: c.can_expand_groups,
            can_fetch_photos: c.can_fetch_photos,
            probed_at: c.probed_at,
        },
        None => GoogleCapabilitiesDto {
            can_expand_groups: None,
            can_fetch_photos: None,
            probed_at: None,
        },
    })
}
