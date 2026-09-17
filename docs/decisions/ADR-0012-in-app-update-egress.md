# ADR-0012: In-app updates via GitHub Releases (second sanctioned non-LLM egress)

Status: Accepted
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
