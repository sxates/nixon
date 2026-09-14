//! Connect-time capability probe (specs/0038 WS3, ADR-0010 best-effort
//! amendment). A granted scope is NOT access, so once the tokens are live we
//! probe ONCE what the account's org actually allows and cache flags on the
//! account row:
//!   - `can_expand_groups` — Cloud Identity group-member listing (DL flatten).
//!   - `can_fetch_photos`  — People-API directory read (attendee photos; the
//!     real fetch is a sibling workstream — this only sets the flag).
//!
//! Everything here is **best-effort and non-fatal**: no probe failure may break
//! connect or sync. Transient/network failures leave the flag unprobed (`NULL`)
//! so a later pass retries — we never latch a transient error to `false`. The
//! probe is stale-gated ([`STALE_AFTER`]) so it runs at connect and then at most
//! weekly, not on every sync.

use chrono::{DateTime, Duration, Utc};
use tauri::{AppHandle, Runtime};

use super::{cloud_identity, people_api, sync};
use crate::database::repositories::google_calendar::GoogleCalendarRepository;
use crate::secrets;

/// Re-probe only when the last probe is older than this (or never ran).
const STALE_AFTER: Duration = Duration::days(7);

/// Probe capabilities only if they've never been probed or the last probe is
/// stale (>7 days). The connect path calls [`probe_and_store`] directly (fresh
/// consent always re-probes); this is the background/startup entry that covers
/// already-connected accounts without a reconnect.
pub async fn probe_if_stale<R: Runtime>(app: &AppHandle<R>) {
    if !super::is_configured() {
        return;
    }
    let Some(pool) = sync::db_pool(app) else {
        return;
    };
    let caps = match GoogleCalendarRepository::get_capabilities(&pool).await {
        Ok(Some(caps)) => caps, // connected
        Ok(None) => return,     // not connected — nothing to probe
        Err(e) => {
            log::warn!("google calendar: could not read capabilities for staleness check: {e}");
            return;
        }
    };
    let fresh = caps
        .probed_at
        .as_deref()
        .and_then(|t| t.parse::<DateTime<Utc>>().ok())
        .map(|t| Utc::now() - t < STALE_AFTER)
        .unwrap_or(false);
    if fresh {
        return;
    }
    probe_and_store(app).await;
}

/// Run the capability probe once and persist the flags. Best-effort by
/// contract: every failure logs and returns; connect/sync never depend on it.
/// Nothing is persisted when the group probe is inconclusive (transient), so
/// the account stays "unprobed" and a later pass retries.
pub async fn probe_and_store<R: Runtime>(app: &AppHandle<R>) {
    if !super::is_configured() {
        return;
    }
    let Some(pool) = sync::db_pool(app) else {
        return;
    };
    // Need a connected account (for the owner email used as a benign probe key).
    let account = match GoogleCalendarRepository::get_account(&pool).await {
        Ok(Some(a)) => a,
        Ok(None) => return,
        Err(e) => {
            log::warn!("google calendar: capability probe could not read the account: {e}");
            return;
        }
    };
    let Some(store) = secrets::store() else {
        log::warn!("google calendar: capability probe skipped — Keychain unavailable");
        return;
    };
    let token = match super::oauth::get_access_token(store).await {
        Ok(t) => t,
        Err(e) => {
            // Auth problems surface through the sync path; the probe just backs off.
            log::warn!("google calendar: capability probe could not get a token: {e:#}");
            return;
        }
    };
    let client = match sync::http_client() {
        Ok(c) => c,
        Err(e) => {
            log::warn!("google calendar: capability probe could not build a client: {e:#}");
            return;
        }
    };

    // Probe on the OWNER's own address rather than a sampled DL (finding #4). A
    // `groups:lookup` on a non-group key still exercises the API's permission
    // gate: a `403` means org policy forbids the Cloud Identity Groups API for
    // this account (an ACCOUNT-level signal), whereas a `403` on one arbitrary
    // DL could just be that one group being invisible — which must NOT poison
    // `can_expand_groups` for the whole org. `404`/`2xx` on the owner key means
    // the API works (the key simply isn't a group).
    let group = cloud_identity::probe_access(&client, &token, &account.email).await;
    let photos = people_api::probe_access(&client, &token).await;

    // Each flag is persisted only when its probe was CONCLUSIVE (`Some`); an
    // inconclusive (`None`, transient) result is never latched to `false` and
    // leaves that column unprobed for a later retry (finding #3). `Some(false)`
    // (a real `403` denial) IS conclusive and stored. `set_capabilities` stamps
    // `capabilities_probed_at` only when at least one flag was written, so a
    // fully inconclusive probe leaves the account "unprobed" and re-probes.
    if group.is_none() && photos.is_none() {
        log::info!("google calendar: capability probe inconclusive; will retry when stale");
        return;
    }
    if let Err(e) =
        GoogleCalendarRepository::set_capabilities(&pool, group, photos, &Utc::now().to_rfc3339())
            .await
    {
        log::warn!("google calendar: could not persist capability probe: {e}");
    } else {
        log::info!(
            "google calendar: capabilities probed (expand_groups={group:?}, fetch_photos={photos:?})"
        );
    }
}
