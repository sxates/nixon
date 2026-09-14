//! Cloud Identity group-member listing (specs/0038 WS3, ADR-0010 best-effort
//! amendment) — the runtime half of distribution-list flattening.
//!
//! Given a live access token (reused from the existing OAuth plumbing —
//! `oauth::get_access_token`, never a fresh auth), this resolves a group
//! address to its Cloud Identity resource name and lists its individual
//! members, so a DL invited to a meeting can be folded into real attendees.
//!
//! **A granted scope is NOT access.** Google returns `403 PERMISSION_DENIED`
//! when org policy forbids the Cloud Identity Groups API for this user even
//! though `cloud-identity.groups.readonly` was consented. Every entry point
//! therefore treats `403` as a *silent* outcome, never a hard error:
//!   - [`lookup_group`] returns a [`LookupOutcome`] that keeps `404` (the
//!     address is definitively NOT a group) distinct from `403` (denied — the
//!     lookup is inconclusive), so the DL-expansion pass only demotes a
//!     confirmed non-group to a person (specs/0038 WS3, finding #1).
//!   - [`list_members`] maps `403` → [`MembersOutcome::Denied`] (distinct from
//!     an empty group), so the caller keeps the labeled-DL floor.
//!   - [`probe_access`] classifies a single lookup as granted/denied/unknown
//!     for the connect-time capability probe.
//!
//! Egress is GETs only (credentials + query), mirroring `sync.rs`; nothing the
//! app produces is ever uploaded.

use anyhow::{bail, Context, Result};
use serde::Deserialize;

const LOOKUP_URL: &str = "https://cloudidentity.googleapis.com/v1/groups:lookup";
/// Base for `{group_name}/memberships` (group_name already contains `groups/`).
const API_BASE: &str = "https://cloudidentity.googleapis.com/v1/";

/// One materialized member of a group — an email plus a display name where the
/// API supplies one (Cloud Identity memberships usually carry only the key).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupMember {
    pub email: Option<String>,
    pub display_name: Option<String>,
}

/// Result of listing a group's members: the members, or a distinct `Denied`
/// (org policy `403`) so the caller degrades to the labeled-DL floor rather
/// than treating "denied" as "empty group" (specs/0038 WS3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MembersOutcome {
    Members(Vec<GroupMember>),
    Denied,
}

/// Result of a group-address lookup (specs/0038 WS3, finding #1). Keeps the
/// three outcomes distinct so the DL-expansion pass can act precisely:
///   - `Group` — a real Cloud Identity group (list its members, keep the flag).
///   - `NotAGroup` (`404`) — the address is definitively NOT a group, so the
///     labeled-DL flag was a false positive and may be cleared (it's a person).
///   - `Denied` (`403`) — org policy forbids *this* lookup; inconclusive, so the
///     flag is left as-is (never demote a group we simply can't see).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LookupOutcome {
    Group(String),
    NotAGroup,
    Denied,
}

/// Defensive cap on paginated Cloud Identity reads so a server returning a
/// repeating `nextPageToken` can't spin forever (finding #7). Far above any
/// real group's page count at the API's max page size.
const MAX_PAGES: usize = 1000;

#[derive(Debug, Deserialize)]
struct LookupResponse {
    /// The group resource name, e.g. `groups/03qcdfon2blah`.
    name: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct MembershipsPage {
    memberships: Vec<Membership>,
    next_page_token: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct Membership {
    preferred_member_key: Option<MemberKey>,
    /// USER | GROUP | SERVICE_ACCOUNT | OTHER — only USER is a person to fold in.
    #[serde(rename = "type")]
    member_type: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct MemberKey {
    id: Option<String>,
}

/// Resolve a group address to a [`LookupOutcome`]: `Group(name)` for a real
/// group, `NotAGroup` for a definitive `404`, or `Denied` for a `403` (org
/// forbids the lookup — inconclusive). All three are silent, non-error outcomes
/// (best-effort contract); only a truly unexpected status is an error.
pub async fn lookup_group(
    client: &reqwest::Client,
    token: &str,
    email: &str,
) -> Result<LookupOutcome> {
    let resp = client
        .get(LOOKUP_URL)
        .query(&[("groupKey.id", email)])
        .bearer_auth(token)
        .send()
        .await
        .context("could not reach Cloud Identity (group lookup)")?;
    match resp.status().as_u16() {
        403 => Ok(LookupOutcome::Denied),
        404 => Ok(LookupOutcome::NotAGroup),
        s if !resp.status().is_success() => {
            bail!("Cloud Identity group lookup failed (HTTP {s})")
        }
        _ => {
            let body: LookupResponse = resp
                .json()
                .await
                .context("unexpected Cloud Identity lookup response")?;
            match body
                .name
                .map(|n| n.trim().to_string())
                .filter(|n| !n.is_empty())
            {
                Some(name) => Ok(LookupOutcome::Group(name)),
                // A 2xx with no resource name isn't a resolvable group.
                None => Ok(LookupOutcome::NotAGroup),
            }
        }
    }
}

/// List a group's USER members (paginated). `403 PERMISSION_DENIED` →
/// [`MembersOutcome::Denied`] (never a hard error); nested groups / service
/// accounts are skipped (only people are folded into a roster).
pub async fn list_members(
    client: &reqwest::Client,
    token: &str,
    group_name: &str,
) -> Result<MembersOutcome> {
    let url = format!("{API_BASE}{group_name}/memberships");
    let mut members: Vec<GroupMember> = Vec::new();
    let mut page_token: Option<String> = None;
    for _ in 0..MAX_PAGES {
        let mut req = client.get(&url).bearer_auth(token);
        if let Some(t) = &page_token {
            req = req.query(&[("pageToken", t.as_str())]);
        }
        let resp = req
            .send()
            .await
            .context("could not reach Cloud Identity (memberships)")?;
        match resp.status().as_u16() {
            403 => return Ok(MembersOutcome::Denied),
            404 => return Ok(MembersOutcome::Members(members)),
            s if !resp.status().is_success() => {
                bail!("Cloud Identity memberships request failed (HTTP {s})")
            }
            _ => {}
        }
        let page: MembershipsPage = resp
            .json()
            .await
            .context("unexpected Cloud Identity memberships response")?;
        for m in page.memberships {
            // Only real people; skip nested groups / service accounts.
            if matches!(m.member_type.as_deref(), Some(t) if t != "USER") {
                continue;
            }
            let email = m
                .preferred_member_key
                .and_then(|k| k.id)
                .map(|id| id.trim().to_string())
                .filter(|id| !id.is_empty());
            if email.is_some() {
                members.push(GroupMember {
                    email,
                    display_name: None,
                });
            }
        }
        // Terminate on an absent OR empty token (an empty string is not a real
        // "more pages" signal); the MAX_PAGES bound guards a repeating token.
        match page.next_page_token {
            Some(t) if !t.is_empty() => page_token = Some(t),
            _ => return Ok(MembersOutcome::Members(members)),
        }
    }
    log::warn!(
        "google calendar: Cloud Identity memberships for '{group_name}' hit the page cap; \
         returning a partial member list"
    );
    Ok(MembersOutcome::Members(members))
}

/// Connect-time capability probe: is the Cloud Identity Groups read API usable
/// for this account? A single `groups:lookup` on `group_key` classifies:
///   - `403 PERMISSION_DENIED` → `Some(false)` (org forbids it).
///   - `2xx`/`404` → `Some(true)` (API works; the key just may not be a group).
///   - network / other status → `None` (inconclusive — leave the flag unprobed
///     so a later pass retries; never latch a transient failure to `false`).
pub async fn probe_access(client: &reqwest::Client, token: &str, group_key: &str) -> Option<bool> {
    let resp = client
        .get(LOOKUP_URL)
        .query(&[("groupKey.id", group_key)])
        .bearer_auth(token)
        .send()
        .await
        .ok()?;
    match resp.status().as_u16() {
        403 => Some(false),
        200 | 404 => Some(true),
        _ => None,
    }
}
