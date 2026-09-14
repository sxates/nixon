//! People API directory client (specs/0038 WS3, ADR-0010 best-effort amendment)
//! — the runtime half of attendee *photos*, the sibling of Cloud Identity DL
//! flattening.
//!
//! Given a live access token (reused from the existing OAuth plumbing —
//! `oauth::get_access_token`, never a fresh auth), this reads the org's domain
//! directory (`people:listDirectoryPeople`) to map each same-org person's email
//! to a profile-photo URL, then downloads those photos into base64 `data:` URIs
//! so rendering is **LOCAL-ONLY** — no render-time egress to Google, and a
//! Disconnect purge removes every byte.
//!
//! **A granted scope is NOT access.** Google returns `403 PERMISSION_DENIED`
//! when org policy forbids the directory read even though `directory.readonly`
//! was consented, so every entry point treats failure as a *silent* outcome:
//!   - [`list_directory_photos`] maps `403` → `Ok(empty map)` (→ initials
//!     fallback for everyone).
//!   - [`fetch_photo_bytes`] maps any error → `Ok(None)` (→ initials for that
//!     one attendee).
//!
//! The directory only holds same-org people, so external attendees simply won't
//! appear in the map and degrade to initials per-attendee. Egress is GETs only
//! (credentials + query), mirroring `sync.rs` / `cloud_identity.rs`.

use std::collections::HashMap;

use anyhow::{bail, Context, Result};
use base64::Engine;
use serde::Deserialize;

use crate::database::repositories::owner_emails::normalize_email;

/// Domain directory listing (same-org profiles only). `readMask` limits the
/// response to the two fields we need; `sources` restricts it to real domain
/// profiles (not the user's private contacts).
const LIST_DIRECTORY_URL: &str = "https://people.googleapis.com/v1/people:listDirectoryPeople";

/// Page size for the directory listing (Google caps this endpoint at 1000; 100
/// keeps each page small while paginating via `nextPageToken`).
const PAGE_SIZE: &str = "100";

/// Defensive cap on paginated directory reads so a server returning a repeating
/// `nextPageToken` can't spin forever (finding #7). At `PAGE_SIZE` = 100 this
/// covers a 100k-person directory — far beyond any realistic org.
const MAX_PAGES: usize = 1000;

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct DirectoryPage {
    people: Vec<Person>,
    next_page_token: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct Person {
    email_addresses: Vec<EmailAddress>,
    photos: Vec<Photo>,
    names: Vec<Name>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct EmailAddress {
    value: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct Name {
    display_name: Option<String>,
    metadata: Option<FieldMetadata>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct FieldMetadata {
    /// Google marks exactly one entry per field as the primary; prefer it.
    primary: bool,
}

impl Person {
    /// This person's best display name: the field Google marks `primary`, else the
    /// first non-empty `displayName`. `None` when the directory carries no usable
    /// name (→ the caller keeps whatever name it already had).
    fn best_name(&self) -> Option<String> {
        let clean = |n: &Option<String>| {
            n.as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
        };
        self.names
            .iter()
            .find(|n| n.metadata.as_ref().map(|m| m.primary).unwrap_or(false))
            .and_then(|n| clean(&n.display_name))
            .or_else(|| self.names.iter().find_map(|n| clean(&n.display_name)))
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct Photo {
    url: Option<String>,
    /// Google's silhouette placeholder flag — skip these (we render initials
    /// ourselves, and a placeholder photo would just hide that).
    default: bool,
}

/// One domain-directory read, keyed by normalized email: profile-photo URLs and
/// display names (specs/0038 WS3). Both come from the SAME paginated
/// `listDirectoryPeople` call (`names,emailAddresses,photos`), so names cost no
/// extra network pass. Either map may be empty (no photo / no name for a person).
#[derive(Debug, Default)]
pub struct DirectoryListing {
    /// normalized email → profile-photo URL (non-placeholder photos only).
    pub photos: HashMap<String, String>,
    /// normalized email → directory display name (RC of "get the name, not just
    /// the email"). Applied only to overwrite an email-fallback name in `sync`.
    pub names: HashMap<String, String>,
}

/// Read the org domain directory once (same-org profiles only) into a
/// [`DirectoryListing`] of `email → photo URL` and `email → display name`,
/// paginating via `nextPageToken`. Placeholder photos (`default: true`) are
/// skipped so real absence falls through to initials.
///
/// Best-effort by contract, never a hard error:
///   - `403 PERMISSION_DENIED` (org forbids the directory read) → the partial
///     listing so far (usually empty) — everyone degrades to initials/email.
///   - `429`/`5xx` (rate limited or a Google-side blip) → a SOFT skip: log and
///     return the partial listing, leaving the existing cache intact so the next
///     scheduled sync retries (RC-3). No backoff machinery — the trigger cadence
///     is the retry.
///
/// External attendees aren't in the domain directory, so they simply won't appear.
pub async fn list_directory(client: &reqwest::Client, token: &str) -> Result<DirectoryListing> {
    let mut listing = DirectoryListing::default();
    let mut page_token: Option<String> = None;
    for _ in 0..MAX_PAGES {
        let mut req = client
            .get(LIST_DIRECTORY_URL)
            .query(&[
                ("readMask", "names,emailAddresses,photos"),
                ("sources", "DIRECTORY_SOURCE_TYPE_DOMAIN_PROFILE"),
                ("pageSize", PAGE_SIZE),
            ])
            .bearer_auth(token);
        if let Some(t) = &page_token {
            req = req.query(&[("pageToken", t.as_str())]);
        }
        let resp = req
            .send()
            .await
            .context("could not reach the Google People directory")?;
        match resp.status().as_u16() {
            // Org policy forbids the directory read — silent, never an error.
            403 => return Ok(listing),
            // Rate limited / transient Google-side error: a soft, non-fatal skip
            // so it can't poison the whole enrichment pass (RC-3). The next
            // scheduled sync re-scans and retries.
            429 | 500..=599 => {
                log::warn!(
                    "google calendar: People directory read returned HTTP {} (transient); \
                     skipping this pass, will retry on the next sync",
                    resp.status().as_u16()
                );
                return Ok(listing);
            }
            s if !resp.status().is_success() => {
                bail!("Google People directory request failed (HTTP {s})")
            }
            _ => {}
        }
        let page: DirectoryPage = resp
            .json()
            .await
            .context("unexpected People directory response from Google")?;
        for person in page.people {
            // The first real (non-default) photo URL, if any, and the best name.
            let url = person
                .photos
                .iter()
                .find(|p| !p.default)
                .and_then(|p| p.url.as_deref())
                .map(|u| u.trim().to_string())
                .filter(|u| !u.is_empty());
            let name = person.best_name();
            if url.is_none() && name.is_none() {
                continue;
            }
            // Map every email address this person carries → that photo/name, keyed
            // through the shared `normalize_email` so the build-side key matches
            // every lookup site exactly (specs/0038 WS3, finding #5).
            for email in &person.email_addresses {
                let key = normalize_email(email.value.as_deref().unwrap_or(""));
                if key.is_empty() {
                    continue;
                }
                if let Some(url) = &url {
                    listing.photos.insert(key.clone(), url.clone());
                }
                if let Some(name) = &name {
                    listing.names.insert(key, name.clone());
                }
            }
        }
        // Terminate on an absent OR empty token; MAX_PAGES guards a repeat.
        match page.next_page_token {
            Some(t) if !t.is_empty() => page_token = Some(t),
            _ => return Ok(listing),
        }
    }
    log::warn!(
        "google calendar: People directory listing hit the page cap; returning a partial map"
    );
    Ok(listing)
}

/// Connect-time capability probe: is the People directory read usable for this
/// account? A single `listDirectoryPeople` (`pageSize=1`, `names` mask — the
/// lightest touch) classifies: `200` → `Some(true)`, `403 PERMISSION_DENIED` →
/// `Some(false)` (org forbids it), any other status/network error → `None`
/// (inconclusive — leave the flag unprobed so a later pass retries; never latch
/// a transient failure to `false`). Mirrors `cloud_identity::probe_access` so
/// the capability probe owns no duplicated People URL/request (reuse cleanup).
pub async fn probe_access(client: &reqwest::Client, token: &str) -> Option<bool> {
    let resp = client
        .get(LIST_DIRECTORY_URL)
        .query(&[
            ("readMask", "names"),
            ("sources", "DIRECTORY_SOURCE_TYPE_DOMAIN_PROFILE"),
            ("pageSize", "1"),
        ])
        .bearer_auth(token)
        .send()
        .await
        .ok()?;
    match resp.status().as_u16() {
        200 => Some(true),
        403 => Some(false),
        _ => None,
    }
}

/// Download one photo URL and return it as a base64 `data:` URI
/// (`data:image/…;base64,…`), so it renders with NO render-time egress to
/// Google (and a Disconnect purge removes it). Returns `Ok(None)` — degrade to
/// initials — for any non-usable response: a network error, a non-2xx status,
/// an empty body, OR a missing/non-`image/*` Content-Type (so an error page or
/// HTML body is never cached as a bogus JPEG — finding #6).
pub async fn fetch_photo_bytes(client: &reqwest::Client, url: &str) -> Result<Option<String>> {
    let resp = match client.get(url).send().await {
        Ok(resp) => resp,
        Err(e) => {
            log::warn!("google calendar: attendee photo fetch failed: {e}");
            return Ok(None);
        }
    };
    if !resp.status().is_success() {
        log::info!(
            "google calendar: attendee photo fetch returned HTTP {}",
            resp.status().as_u16()
        );
        return Ok(None);
    }
    // Content-type before consuming the body (the header borrows `resp`). Only
    // a real `image/*` type is usable; anything else (missing, HTML error page)
    // means there's no photo here — degrade to initials rather than cache junk.
    let Some(content_type) = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.split(';').next().unwrap_or(s).trim().to_string())
        .filter(|s| s.starts_with("image/"))
    else {
        log::info!("google calendar: attendee photo response was not an image; skipping");
        return Ok(None);
    };
    let bytes = match resp.bytes().await {
        Ok(b) if !b.is_empty() => b,
        Ok(_) => return Ok(None), // empty body ⇒ no photo
        Err(e) => {
            log::warn!("google calendar: attendee photo body read failed: {e}");
            return Ok(None);
        }
    };
    let encoded = base64::engine::general_purpose::STANDARD.encode(&bytes);
    Ok(Some(format!("data:{content_type};base64,{encoded}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The directory-map parse: first non-default photo per person, every email
    /// lowercased, placeholder (`default:true`) photos and people with no real
    /// photo dropped.
    #[test]
    fn directory_page_parses_email_to_photo_map() {
        let body = r#"{
            "people": [
                {
                    "emailAddresses": [ { "value": "Alice@Example.com" } ],
                    "photos": [
                        { "url": "https://lh3.google/default", "default": true },
                        { "url": "https://lh3.google/alice", "default": false }
                    ]
                },
                {
                    "emailAddresses": [ { "value": "bob@example.com" }, { "value": "b@example.com" } ],
                    "photos": [ { "url": "https://lh3.google/bob" } ]
                },
                {
                    "emailAddresses": [ { "value": "noface@example.com" } ],
                    "photos": [ { "url": "https://lh3.google/placeholder", "default": true } ]
                }
            ],
            "nextPageToken": null
        }"#;
        let page: DirectoryPage = serde_json::from_str(body).unwrap();

        // Reproduce the aggregation the paginating loop performs for one page.
        let mut map: HashMap<String, String> = HashMap::new();
        for person in page.people {
            let Some(url) = person
                .photos
                .into_iter()
                .find(|p| !p.default)
                .and_then(|p| p.url)
            else {
                continue;
            };
            for email in person.email_addresses {
                let key = normalize_email(email.value.as_deref().unwrap_or(""));
                if !key.is_empty() {
                    map.insert(key, url.clone());
                }
            }
        }

        assert_eq!(
            map.get("alice@example.com").unwrap(),
            "https://lh3.google/alice"
        );
        assert_eq!(
            map.get("bob@example.com").unwrap(),
            "https://lh3.google/bob"
        );
        assert_eq!(map.get("b@example.com").unwrap(), "https://lh3.google/bob");
        assert!(
            !map.contains_key("noface@example.com"),
            "a person with only a default placeholder photo is skipped"
        );
        assert_eq!(map.len(), 3);
    }

    /// `best_name` prefers the entry Google marks `primary`; with none marked it
    /// takes the first non-empty display name; with no usable names it's `None`.
    #[test]
    fn best_name_prefers_primary_then_first() {
        let primary: Person = serde_json::from_str(
            r#"{ "names": [
                { "displayName": "Nick" },
                { "displayName": "Priya Patel", "metadata": { "primary": true } }
            ] }"#,
        )
        .unwrap();
        assert_eq!(primary.best_name().as_deref(), Some("Priya Patel"));

        let first: Person = serde_json::from_str(
            r#"{ "names": [ { "displayName": "  Sam Lee  " }, { "displayName": "Samuel" } ] }"#,
        )
        .unwrap();
        assert_eq!(
            first.best_name().as_deref(),
            Some("Sam Lee"),
            "no primary ⇒ first non-empty name, trimmed"
        );

        let none: Person =
            serde_json::from_str(r#"{ "names": [ { "displayName": "   " } ] }"#).unwrap();
        assert_eq!(none.best_name(), None, "blank names ⇒ None");

        let empty: Person = serde_json::from_str(r#"{ "emailAddresses": [] }"#).unwrap();
        assert_eq!(empty.best_name(), None, "no names field ⇒ None");
    }
}
