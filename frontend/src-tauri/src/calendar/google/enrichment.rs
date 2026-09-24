//! Best-effort enrichment after a sync pass (specs/0038 WS3; moved off the sync
//! lock in specs/0074 W1): distribution-list expansion via Cloud Identity and
//! the attendee photo pass ([`super::photos`]).
//!
//! [`spawn_enrichment`] runs after the event phase has released `SYNC_LOCK`, so
//! a slow directory read can never make "Sync now" or an agenda build wait. It
//! is single-flighted by its own [`ENRICH_LOCK`]; a pass that finds it busy is
//! skipped (the next sync spawns another). Each half is gated on its probed
//! capability and is wholly non-fatal.

use std::collections::HashSet;
use std::time::Instant;

use anyhow::{Context, Result};
use serde_json::json;
use sqlx::SqlitePool;
use tauri::{AppHandle, Runtime};

use super::photos::fetch_attendee_photos;
use super::{cloud_identity, sync};
use crate::database::repositories::google_calendar::GoogleCalendarRepository;
use crate::database::repositories::owner_emails::OwnerEmailsRepository;

static ENRICH_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Spawn the enrichment pass with the sync pass's access token. Never blocks
/// the caller; a pass already running makes this one a no-op.
pub(super) fn spawn_enrichment<R: Runtime>(app: &AppHandle<R>, token: String) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let Ok(_guard) = ENRICH_LOCK.try_lock() else {
            log::info!("google calendar: enrichment pass already running; skipping");
            return;
        };
        let Some(pool) = sync::db_pool(&app) else {
            return;
        };
        run_enrichment(&pool, &token).await;
    });
}

async fn run_enrichment(pool: &SqlitePool, token: &str) {
    let caps = GoogleCalendarRepository::get_capabilities(pool)
        .await
        .ok()
        .flatten();
    let can_expand = caps.as_ref().and_then(|c| c.can_expand_groups) == Some(true);
    let can_photos = caps.as_ref().and_then(|c| c.can_fetch_photos) == Some(true);
    if !can_expand && !can_photos {
        return;
    }
    let client = match sync::http_client() {
        Ok(c) => c,
        Err(e) => {
            log::warn!("google calendar: enrichment skipped (no HTTP client): {e:#}");
            return;
        }
    };
    let started = Instant::now();

    // DL flattening: only when the org granted Cloud Identity access. One
    // group's failure never aborts the pass; the labeled-DL floor stands.
    let mut dl_groups = 0usize;
    if can_expand {
        let owner_emails: HashSet<String> = OwnerEmailsRepository::list(pool)
            .await
            .unwrap_or_default()
            .into_iter()
            .collect();
        match expand_distribution_lists(pool, &client, token, &owner_emails).await {
            Ok(n) => dl_groups = n,
            Err(e) => log::warn!("google calendar: DL expansion pass failed (floor stands): {e:#}"),
        }
    }

    // Attendee photos: only when the org granted the People directory read.
    // Any failure degrades to initials.
    let photos = if !can_photos {
        "off"
    } else if let Err(e) = fetch_attendee_photos(pool, &client, token).await {
        log::warn!("google calendar: attendee photo pass failed (initials stand): {e:#}");
        "failed"
    } else {
        "ok"
    };
    log::info!(
        "google calendar: enrichment pass dl_groups={dl_groups} photos={photos} duration_ms={}",
        started.elapsed().as_millis()
    );
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
///
/// Returns how many DL addresses were resolved this pass.
async fn expand_distribution_lists(
    pool: &SqlitePool,
    client: &reqwest::Client,
    token: &str,
    owner_emails: &HashSet<String>,
) -> Result<usize> {
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
    Ok(resolved.len())
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
