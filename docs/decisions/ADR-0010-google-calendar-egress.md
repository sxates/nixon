# ADR-0010: Opt-in, read-only Google Calendar API egress

Status: Accepted (owner signed off on the unverified-app posture, 2026-07-02)
Date: 2026-07-01

## Context

Vinyl's calendar source is **EventKit only** (`frontend/src-tauri/src/calendar/eventkit.rs`)
— zero network egress, matching the privacy posture in `CLAUDE.md` where the *only*
sanctioned outbound traffic is the user-chosen LLM provider. Three releases of feedback
say EventKit caps calendar quality:

- **Distribution lists can't be expanded** (`specs/0027`): EventKit hands a DL to us as
  one opaque `EKParticipant` — the roster shows one fake "person". 0027 is explicitly
  deferred on a real Google Calendar connection.
- **Attendee/organizer fidelity is limited** — no RSVP status, no organizer, names often
  missing; this caps People (`specs/0016`), participants (`specs/0017`), and everything
  in 0013 Wave 4.
- **EventKit identifiers are unreliable**: provider syncs reissue `eventIdentifier`
  (the 0029 WS6.2 dismissal-key bug); the Google API's ids and `iCalUID` are stable at
  the source.
- EventKit only works if the account is added to macOS Calendar and access granted; a
  direct connection is consistent across machines.

This is the **first sanctioned non-LLM egress**, so the decision — not the code — is the
pattern-setter for every future cloud integration (Zoom OAuth is already queued behind
the same shape of decision).

## Decision

Allow an **opt-in, read-only** Google Calendar API integration (`specs/0032`), with these
boundaries:

**Egress boundary — metadata ingest only.** The only Google traffic is: the OAuth
endpoints (`accounts.google.com`, `oauth2.googleapis.com`) and **GET**s to the Calendar
API (`www.googleapis.com/calendar/v3/...`). Requests carry OAuth credentials and query
parameters only. Meeting **audio, transcripts, notes, summaries, titles, and every other
app-generated artifact never leave the machine** — this integration downloads event
metadata; it uploads nothing. Verified by packet capture (the `specs/0013` precedent),
and re-verified whenever the integration's request surface changes.

**Scopes — Calendar only, granular, read-only.**
`https://www.googleapis.com/auth/calendar.events.readonly` +
`https://www.googleapis.com/auth/calendar.calendarlist.readonly` (the narrowest granular
pair that supports "list my calendars, read events with attendees"; not the broader
`calendar.readonly`). On DL expansion, precisely: the **Calendar API alone does not
deterministically expand a group** — an invited group appears as a single attendee (no
"is a group" flag), but individual members **materialize as separate attendees when they
RSVP**, and Google performs server-side member listing for group attendees (surfaced by
`attendees[].asyncOperation`, "listing of members of large attendee groups"). That is
already a large fidelity win over EventKit's single opaque participant. Deterministic
full-membership expansion requires the **Admin SDK Directory API**
(`admin.directory.group.member.readonly`) or the **Cloud Identity Groups API**
(`cloud-identity.groups.readonly`) — Workspace-only, dependent on org group-visibility
policy; the People API does *not* expand Workspace DLs (personal contact groups only).
**Directory/Cloud Identity/People scopes are OUT for v1**; revisit as a v2 only if
Calendar-level attendee fidelity proves insufficient on real meetings.

**Token storage — Keychain per ADR-0009.** Refresh token stored via the `SecretStore`
layer (`frontend/src-tauri/src/secrets.rs`), service = bundle identifier (dev/prod
isolated per ADR-0004), account `gcal.refresh_token`. Access tokens live in memory only.
**No sentinel/DB fallback for OAuth tokens** — unlike API keys, a token must never be
written to SQLite or logs under any failure mode; a failed Keychain write fails the
connect. Non-secret state (account email, calendar selection, sync tokens, event cache)
lives in SQLite.

**OAuth client posture — desktop-app PKCE loopback.** A Google Cloud "Desktop app" OAuth
client; authorization-code flow with **PKCE (S256)** and a **loopback redirect**
(`http://127.0.0.1:<ephemeral port>`). Google issues desktop clients a `client_secret`
that its own docs designate **not confidential** — it is baked into the binary alongside
the client id (build-time env, `specs/0032`); PKCE is the actual protection.
**Consent-screen posture (owner to confirm):** publish the consent screen **"In
production" but unverified** — users see Google's "unverified app" interstitial once
(Advanced → Continue), capped at 100 users; crucially this avoids Testing status's 7-day
refresh-token expiry. Google's verification process (required because Calendar scopes are
classified *sensitive*) is undertaken **only if/when Vinyl is publicly distributed**.
Until then this is a personal-use integration against Brian's own GCP project.

**Single active source — EventKit stays the zero-config default.** Google is opt-in;
while connected it is the **only** calendar source (amended 2026-07-02 from an earlier
merged-view design — the owner chose one source at a time over cross-source dedupe
complexity). Dismissal keys are computed identically for both sources via the shared
external identity (`iCalUID` ↔ EventKit `calendarItemExternalIdentifier`), so hidden
events survive switching sources. **Disconnect** best-effort revokes the token
(`oauth2.googleapis.com/revoke`), deletes Keychain items, purges the local event cache,
and returns the app to EventKit-only with zero Google egress thereafter.

## Consequences / caveats

- Vinyl's privacy story gains a clause: "read-only calendar metadata is *downloaded* from
  Google when — and only when — you connect it." Settings copy must say exactly this.
- This ADR is the template for future integrations (Zoom): opt-in, narrowest read scope,
  Keychain tokens, packet-capture-verified egress boundary, clean disconnect.
- The 100-user cap and unverified interstitial are acceptable for personal use but block
  public distribution — verification (weeks, requires privacy policy + domain) becomes a
  release gate if that changes.
- ADR-0009's dev-build caveat applies: ad-hoc-signed dev rebuilds can lose Keychain
  access → the dev app degrades to "not connected" (reconnect in Settings), never an
  error loop.
- DL expansion quality is now bounded by Google's behavior (RSVP materialization +
  server-side group listing), not by us; if real-world rosters still show opaque DLs, the
  v2 decision is a *new* scope grant (Directory/Cloud Identity), i.e. an amendment here.
- Rejected alternatives: **CalDAV** (no attendee-expansion advantage over EventKit, and
  Google's CalDAV needs OAuth anyway); **Contacts-permission local group expansion**
  (0027 Phase 2 — only covers user-defined local groups); **webhook push** (needs a
  public HTTPS endpoint — impossible for a local-first desktop app; we poll).

---

## Amendment 1 — WS3 attendee fidelity via optimistic, best-effort scope use

Status: **Accepted** (owner chose the optimistic + graceful-fallback posture, 2026-07-07).
Date: 2026-07-07. Raised by: `specs/0038` WS3 (P3). Supersedes an earlier "pick one A × one
B scope grant" draft — the resolution is neither "hold the line" nor a hard scope commitment,
but **request the extra scopes and use them best-effort, degrading silently when the account
or org can't support them.**

### Decision

Widen the Google grant by **two additional read-only scopes**, both classified **sensitive**
(not *restricted* — no CASA/security assessment), used **best-effort**:

- `https://www.googleapis.com/auth/cloud-identity.groups.readonly` — flatten distribution-list
  (Google Group) membership.
- `https://www.googleapis.com/auth/directory.readonly` — attendee profile photos (org directory).

**Admin SDK Directory (`admin.directory.group.member.readonly`) is explicitly NOT adopted.**
Research (2026-07-07) confirmed it is **admin-only** — a non-admin member gets `403` — so it
buys nothing for our user and adds an admin-gated scope for no gain. Cloud Identity is the
only non-admin-viable group-listing path.

### Capability model — optimistic, probed, per-item fallback

1. **Floor (always; the fallback, built unconditionally):** a DL shows the group **labeled**
   ("Distribution list — members added as they RSVP") plus whichever members already RSVP'd
   (Calendar returns those), and keeps the manual **"Add members"** hatch (0027 Phase 1/3);
   the DL entry is **excluded from speaker/voiceprint binding**. Avatars are initials. An
   account with no enhanced access sees exactly this, with **zero extra egress**.
2. **Probe at connect:** after OAuth, one lightweight attempt per capability
   (`groups.memberships.list` on one group, `people.listDirectoryPeople` once); cache
   `can_expand_groups` / `can_fetch_photos` with a timestamp, re-probed occasionally, **not**
   per-sync.
3. **Runtime, per-item degradation — a grant is not access** (org policy can still `403`):
   - **DL flattening (Cloud Identity):** resolve group email → id (`groups.lookup`) →
     `groups.memberships.list`. Succeeds for a non-admin **iff** that group's *"Who can view
     members"* allows members/the org; a locked-down group returns `PERMISSION_DENIED` and
     **that DL** falls back to the floor. Per-group, best-effort — some groups expand, others
     don't, no failure is fatal.
   - **Photos (People API):** the directory is domain-scoped and `getBatchGet` won't accept
     raw emails, so build an email→`people/{id}` map via `people.listDirectoryPeople`
     (cached), then read `photos`. Resolves for **same-org** attendees; **external**
     attendees have no directory photo → initials. Per-attendee, best-effort.

### Egress & privacy

- Egress boundary now includes `cloudidentity.googleapis.com` and `people.googleapis.com`,
  **GETs only**, carrying credentials + query params only — **still downloads metadata,
  uploads nothing.** Re-verify by packet capture (the `specs/0013` precedent) before shipping;
  update the Settings privacy copy to name exactly what's downloaded.
- **Zero extra egress on unsupported accounts:** after a negative probe the app makes no
  further calls to that API.
- Scopes requested **additively**; **Disconnect still purges everything** — token revoke +
  Keychain + **all** caches, including any member/photo cache. Single-active-source model and
  Keychain token storage unchanged.
- Enhanced data cached on-device like the event cache: expanded members folded into
  `attendees_json` / the roster; photos in a small `attendee_photos(email, blob_or_url,
  fetched_at)` cache.

### Verification posture — marginal cost is ~nil

The base ADR already ships **In-production-but-unverified with sensitive Calendar scopes**,
accepting the unverified-app interstitial and the **permanent 100-user cap** on the OAuth
project. Adding two more *sensitive* scopes **does not change that posture** — same
interstitial, same (already-permanent) cap, and **no** CASA (these are sensitive, not
restricted). Marginal costs are only: two more lines on the consent screen, and the target
**Workspace admin may block the app or either scope** by API-access policy — which surfaces as
the same graceful fallback, not an error. Public distribution would still require Google
**sensitive-scope** verification (justification + demo video to Trust & Safety, ~days–weeks),
unchanged in kind from the Calendar scopes already in play.

### Consent UX

Request both scopes **at connect** (seamless "just works" — chosen for a personal-use tool),
not behind an opt-in toggle. The incremental-auth toggle remains the privacy-conservative
alternative if consent-screen minimalism later matters.

### Follow-through checklist (build)

- [ ] `specs/0038` WS3 re-scoped from "A0/A1/A2 × B0/B1 decision" to "optimistic best-effort
      implementation" per this amendment.
- [ ] Cloud Identity + People API clients under `calendar/google/`; connect-time probe;
      capability flags persisted.
- [ ] Floor behavior (0027 DL label + exclude-binding + manual add) built first as the fallback.
- [ ] Packet-capture re-verification of the egress boundary; Settings privacy copy updated;
      Settings surfaces capability availability honestly ("Enhanced attendee details:
      available / not available on this account").
