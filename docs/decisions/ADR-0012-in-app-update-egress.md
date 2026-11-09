# ADR-0012: In-app updates (second sanctioned non-LLM egress) — amended 2026-10-04: now served from nixonapp.com; 2026-11-08: early access feed

Status: Accepted (amended 2026-10-04 and 2026-11-08, see Amendments)
Date: 2026-09-16

## Context

Nixon's privacy posture (README, PRIVACY_POLICY) is that meeting data never leaves the
machine and the only outbound traffic is to the user-chosen LLM provider, plus the
opt-in Google Calendar connection (ADR-0010). Users who are not developers have no way to
learn about, let alone install, a new version without revisiting the GitHub release page.

## Decision

The app checks for updates on its own and downloads them in the background
(`tauri-plugin-updater`, specs/0058):

- **Destination:** `github.com` only — one GET for the release manifest
  (`releases/latest/download/latest.json`) and one GET for the signed
  `Nixon.app.tar.gz`. No other host, no telemetry endpoint.
- **What is sent:** nothing about the user or their meetings. The requests carry the
  stock HTTP user agent and the IP address GitHub sees for any download.
- **When:** ~20 s after launch, then every 6 hours, while "Download updates
  automatically" (Settings > General) is on. It is on by default; turning it off stops
  all unattended network activity. "Check for updates" in About is always manual.
- **Verification:** every payload is signed with a minisign key whose public half is
  compiled into the app; the plugin refuses anything else. This is independent of
  Apple's Developer ID signature and notarization, which the payload also carries.
- **Never automatic:** installing and restarting is always a click, and is refused
  while a recording is in progress.

## Consequences

- Losing the update signing private key strands every installed copy on manual updates.
  The key lives outside the repo and must be backed up by the release owner.
- A GitHub release marked "pre-release" is invisible to the updater (escape hatch).
- Installs that pre-date this ADR (0.2.x) cannot self-update; one last manual install.
- PRIVACY_POLICY.md and README.md name this traffic explicitly.

## Amendment 2026-10-04 (specs/0080)

The update feed moves from `github.com` to `nixonapp.com` (Cloudflare), so the repository
can eventually go private without breaking updates.

- **Destination:** `nixonapp.com` instead of `github.com`. The manifest
  (`/updates/latest.json`) and the signed `Nixon.app.tar.gz` are served from there.
- **What is sent:** the manifest request now carries the running app version and operating system (`darwin`)
  in the URL query (`?current_version=…&target=…`). It is still a GET with no body and no
  identifiers: no account, device or meeting information.
- **What is recorded:** the Worker records only the event type, app version and platform with a
  timestamp (Workers Analytics Engine), counted per those fields and aggregated when read. No per-user record is kept.
- **Legacy installs:** the GitHub endpoint remains only for installs that predate the
  cutover; they hop to the new feed on their next update. Release publishing continues to
  mirror to GitHub until those have drained.
- Verification (minisign), cadence, the opt-out and the never-automatic install are unchanged.

## Amendment 2026-11-08 (specs/0086)

A second manifest on the same host, chosen by a user setting.

- **Destination:** unchanged host. With **Early access updates** (Settings > General >
  Updates, off by default) turned on, the manifest request goes to `nixonapp.com/updates/early.json`
  instead of `/updates/latest.json`. The payload URLs it advertises are the same
  `nixonapp.com/releases/…` paths. With it off, the app uses the endpoint in
  `tauri.conf.json` exactly as before. The URL lives in `updater/feed.rs` (`EARLY_FEED`),
  and a unit test pins it to the configured endpoint so the two cannot drift apart.
- **What is sent:** unchanged — the same GET with the same version and platform query. The
  Worker counts early-feed checks under their own event name; no new fields are recorded.
- **When:** unchanged. The switch does not add network activity of its own: turning it on
  checks right away only if "Download updates automatically" is on, mirroring that toggle.
- **Verification:** unchanged. Early builds are signed with the same minisign key and
  notarized like any release.
- **Turning it off never downgrades:** the plugin only offers versions strictly newer than
  the installed one. An update already downloaded, which may be an early build, is
  discarded and the regular feed is checked again, so an opted-out machine never installs an
  early build at its next restart.
