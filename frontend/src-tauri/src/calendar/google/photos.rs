//! Attendee photo enrichment (specs/0038 WS3; extracted from `sync.rs` in
//! specs/0056 W6 as a no-behaviour-change move) — the best-effort pass that
//! reads the Google People directory once per sync and caches same-org profile
//! photos into `attendee_photos` as base64 `data:` URIs.
//!
//! Called from [`super::sync`] at the end of a sync pass, only when the org
//! grants the directory read (`can_fetch_photos`). Every failure degrades to
//! initials for that attendee and never touches the sync result.

use anyhow::{Context, Result};
use chrono::{DateTime, Duration, Utc};
use sqlx::SqlitePool;

use super::people_api;
use super::sync::redact_email;
use crate::database::repositories::attendee_photos::AttendeePhotosRepository;
use crate::database::repositories::google_calendar::GoogleCalendarRepository;
use crate::database::repositories::owner_emails::normalize_email;
use crate::database::repositories::people::{PeopleRepository, Person};

/// Cached attendee photos are re-fetched once they're older than this — a
/// person's profile photo changes rarely, so 30 days keeps egress minimal while
/// still refreshing (specs/0038 WS3).
const PHOTO_TTL_DAYS: i64 = 30;

/// specs/0056 W6 review: the wanted set now includes every person with an email, not just the
/// calendar window, so the first pass after upgrade could otherwise burst hundreds of photo
/// downloads in one sync. Fetch at most this many per pass; later passes catch up the rest.
const MAX_PHOTOS_PER_PASS: usize = 40;

/// Fetch same-org profile photos into the local cache, when the org grants the
/// People directory read (`can_fetch_photos`). Best-effort and the sibling of
/// `sync::expand_distribution_lists`: it reads the domain directory once per
/// pass (`email → url`), then downloads bytes as base64 `data:` URIs for every
/// wanted email that isn't already freshly cached (<30 days).
///
/// The wanted set (specs/0056 W6) is **cached-event attendees ∪ people with an
/// email** ([`photo_wanted_emails`]), so a person who isn't in the current
/// calendar window still gets their photo — the People directory and person
/// page render it through `PeopleRepository`'s `attendee_photos` join. External
/// addresses simply aren't in the directory (→ initials); every failure degrades
/// to initials for that email and never touches the sync result.
///
/// The directory listing is one paginated metadata call; the heavy part (photo
/// bytes) is cached with a 30-day TTL, so steady-state egress is just the small
/// directory read per pass — and ZERO reads when nothing is stale and no name is
/// upgradable. Rendering is LOCAL-ONLY — a `data:` URI, never a Google URL — so
/// a Disconnect purge removes every byte.
pub(super) async fn fetch_attendee_photos(
    pool: &SqlitePool,
    client: &reqwest::Client,
    token: &str,
) -> Result<()> {
    // The small app-wide people set, loaded ONCE: it widens the wanted set below
    // and feeds the directory-name upgrade further down.
    let people = PeopleRepository::list(pool).await.unwrap_or_default();

    let wanted = photo_wanted_emails(pool, &people)
        .await
        .context("could not read cached attendees for photo enrichment")?;
    if wanted.is_empty() {
        return Ok(());
    }

    // Compute the stale/absent photo subset via ONE bulk `fetched_at` read
    // (finding #9 — no per-email `get_photo`).
    let now = Utc::now();
    let freshness = AttendeePhotosRepository::fetched_at_map(pool)
        .await
        .context("could not read cached attendee photo freshness")?;
    let stale: Vec<String> = wanted
        .iter()
        .filter(|key| match freshness.get(*key) {
            Some(fetched_at) => fetched_at
                .parse::<DateTime<Utc>>()
                .ok()
                .map(|t| now - t >= Duration::days(PHOTO_TTL_DAYS))
                .unwrap_or(true), // unparseable stamp ⇒ treat as stale
            None => true, // absent ⇒ needs a fetch
        })
        .cloned()
        .collect();

    // People still carrying an auto-derived email-fallback name (specs/0038 WS3:
    // "get the name, not just the email"). Keep only those whose email is wanted
    // AND whose stored name is clearly the email fallback — the SAFE-to-overwrite
    // set. If it's empty AND no photos are stale, the directory read below is
    // skipped, so a steady-state pass still makes ZERO People API calls (finding #8).
    let upgradable: Vec<Person> = people
        .into_iter()
        .filter(|p| {
            p.email
                .as_deref()
                .map(normalize_email)
                .filter(|e| !e.is_empty())
                .map(|e| wanted.contains(&e) && is_email_fallback_name(&p.display_name, &e))
                .unwrap_or(false)
        })
        .collect();

    if stale.is_empty() && upgradable.is_empty() {
        return Ok(()); // everything fresh + every name real — no directory read, no egress
    }

    // The domain directory read (ONE paginated call): normalized email → photo URL
    // and → display name. Empty when the org forbids the read or a transient
    // 429/5xx soft-skipped it (RC-3) — then everyone keeps initials/their name.
    let directory = people_api::list_directory(client, token)
        .await
        .context("could not read the Google People directory")?;
    if directory.photos.is_empty() && directory.names.is_empty() {
        return Ok(());
    }

    // Names first: overwrite ONLY email-fallback names, and only when the
    // directory name differs and isn't itself just the email (never clobber a
    // real/user-edited name — that's guaranteed by the `upgradable` filter above).
    let mut renamed = 0usize;
    for person in upgradable {
        let Some(email) = person.email.as_deref() else {
            continue;
        };
        let key = normalize_email(email);
        let Some(dir_name) = directory.names.get(&key) else {
            continue; // external / no directory name → keep the fallback
        };
        if dir_name == &person.display_name || is_email_fallback_name(dir_name, &key) {
            continue;
        }
        match PeopleRepository::update(
            pool,
            &person.id,
            dir_name,
            person.email.as_deref(),
            person.role.as_deref(),
            person.notes.as_deref(),
        )
        .await
        {
            Ok(true) => renamed += 1,
            Ok(false) => {} // person vanished mid-pass — harmless
            Err(e) => log::warn!(
                "google calendar: could not apply a directory name ({}): {e}",
                redact_email(&key)
            ),
        }
    }
    if renamed > 0 {
        log::info!("google calendar: upgraded {renamed} attendee name(s) from the directory");
    }

    let mut fetched = 0usize;
    // Bounded per pass (see `MAX_PHOTOS_PER_PASS`); only same-org emails with a directory
    // photo count against the cap, so external attendees never crowd it out.
    let fetchable = stale
        .into_iter()
        .filter(|key| directory.photos.contains_key(key))
        .take(MAX_PHOTOS_PER_PASS);
    for key in fetchable {
        let Some(url) = directory.photos.get(&key) else {
            continue; // external attendee / no same-org photo → initials
        };
        match people_api::fetch_photo_bytes(client, url).await {
            Ok(Some(data_uri)) => {
                if let Err(e) =
                    AttendeePhotosRepository::upsert_photo(pool, &key, &data_uri, &now.to_rfc3339())
                        .await
                {
                    log::warn!(
                        "google calendar: could not cache an attendee photo ({}): {e}",
                        redact_email(&key)
                    );
                } else {
                    fetched += 1;
                }
            }
            Ok(None) => {} // no usable photo bytes → initials for this attendee
            Err(e) => log::warn!(
                "google calendar: attendee photo fetch failed ({}): {e:#}",
                redact_email(&key)
            ),
        }
    }
    if fetched > 0 {
        log::info!("google calendar: cached {fetched} attendee photo(s)");
    }
    Ok(())
}

/// True when `display_name` is clearly the auto-derived email fallback for
/// `email` — equal to the whole address, or to the address's local-part, both
/// compared case-insensitively (specs/0038 WS3). This is the ONLY case the
/// directory-name upgrade is allowed to overwrite: a name that differs from both
/// is a real calendar `displayName` or a user edit and is never clobbered.
fn is_email_fallback_name(display_name: &str, email: &str) -> bool {
    let name = display_name.trim().to_ascii_lowercase();
    let email = email.trim().to_ascii_lowercase();
    if name.is_empty() || email.is_empty() {
        return false;
    }
    if name == email {
        return true;
    }
    match email.split_once('@') {
        Some((local, _)) => !local.is_empty() && name == local,
        None => false,
    }
}

/// The photo work list (specs/0056 W6): every distinct normalized email across
/// the cached events' attendees, UNION every `people` row's email (normalized,
/// blanks dropped). The union is what lets a person who isn't a recent attendee
/// still get their directory photo on the next pass.
async fn photo_wanted_emails(
    pool: &SqlitePool,
    people: &[Person],
) -> Result<std::collections::HashSet<String>> {
    let mut wanted: std::collections::HashSet<String> = distinct_cached_attendee_emails(pool)
        .await?
        .into_iter()
        .collect();
    wanted.extend(
        people
            .iter()
            .filter_map(|p| p.email.as_deref())
            .map(normalize_email)
            .filter(|e| !e.is_empty()),
    );
    Ok(wanted)
}

/// The distinct (lowercased) attendee emails across every cached event — the
/// photo-fetch work list. Reads each row's `attendees_json` and collects the
/// `email` fields; malformed rows are skipped (best-effort).
async fn distinct_cached_attendee_emails(pool: &SqlitePool) -> Result<Vec<String>> {
    let blobs = GoogleCalendarRepository::all_attendees_json(pool).await?;
    let mut set: std::collections::HashSet<String> = std::collections::HashSet::new();
    for blob in blobs {
        let parsed: Vec<serde_json::Value> = match serde_json::from_str(&blob) {
            Ok(v) => v,
            Err(_) => continue,
        };
        for a in parsed {
            if let Some(email) = a.get("email").and_then(|v| v.as_str()) {
                let norm = normalize_email(email);
                if !norm.is_empty() {
                    set.insert(norm);
                }
            }
        }
    }
    Ok(set.into_iter().collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::repositories::google_calendar::GoogleCalendarEventRow;
    use sqlx::sqlite::SqlitePoolOptions;

    /// In-memory pool through the app's REAL migration set (one connection — each
    /// in-memory connection is a separate database).
    async fn test_pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("open in-memory sqlite");
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        pool
    }

    /// specs/0056 W6: the photo work list is cached attendees ∪ people with an email, all
    /// normalized — a person who isn't in the calendar window (or whose email is cased
    /// differently) still gets a photo on the next pass. Email-less people contribute nothing.
    #[tokio::test]
    async fn wanted_set_is_cached_attendees_union_people_with_email() {
        let pool = test_pool().await;

        // One cached event with one attendee.
        GoogleCalendarRepository::upsert_event(
            &pool,
            &GoogleCalendarEventRow {
                id: "gcal:primary/e1".into(),
                calendar_id: "primary".into(),
                ical_uid: None,
                title: "Standup".into(),
                starts_at: "2026-09-01T09:00:00+00:00".into(),
                ends_at: "2026-09-01T09:30:00+00:00".into(),
                is_all_day: false,
                location: None,
                zoom_url: None,
                organizer_email: None,
                my_response: Some("accepted".into()),
                attendees_json: r#"[{"email":"Amy@Example.com","displayName":"Amy"}]"#.into(),
                status: "confirmed".into(),
                updated_at: "2026-09-01T00:00:00+00:00".into(),
            },
        )
        .await
        .unwrap();

        // A person who is NOT a cached attendee, plus one with no email at all.
        PeopleRepository::create(&pool, "Zed", Some(" Zed@Example.com "), None, None)
            .await
            .unwrap();
        PeopleRepository::create(&pool, "Nobody", None, None, None)
            .await
            .unwrap();
        let people = PeopleRepository::list(&pool).await.unwrap();

        let wanted = photo_wanted_emails(&pool, &people).await.unwrap();
        let mut got: Vec<&str> = wanted.iter().map(String::as_str).collect();
        got.sort_unstable();
        assert_eq!(got, vec!["amy@example.com", "zed@example.com"]);
    }

    #[test]
    fn email_fallback_name_upgrades_only_derived_names() {
        // The auto-derived fallbacks map_event produces — SAFE to overwrite.
        assert!(is_email_fallback_name(
            "priya@example.com",
            "priya@example.com"
        ));
        assert!(is_email_fallback_name(
            "Priya@Example.com",
            "priya@example.com"
        ));
        assert!(is_email_fallback_name("priya", "priya@example.com")); // local-part fallback
        assert!(is_email_fallback_name("PRIYA", "priya@example.com"));
        // A real name / user edit that differs from both the address and its
        // local-part — NEVER clobbered (the safety invariant).
        assert!(!is_email_fallback_name("Priya Patel", "priya@example.com"));
        assert!(!is_email_fallback_name("Priya P.", "priya@example.com"));
        // A different local-part than the derived one is also a real signal.
        assert!(!is_email_fallback_name("ppatel", "priya@example.com"));
        // Degenerate inputs never trigger an overwrite.
        assert!(!is_email_fallback_name("", "priya@example.com"));
        assert!(!is_email_fallback_name("priya", ""));
    }
}
