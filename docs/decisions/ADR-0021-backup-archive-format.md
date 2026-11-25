# ADR-0021: The backup archive is a plain zip with an allowlisted layout

Status: Accepted
Date: 2026-11-24

## Context

Nothing gathered Nixon's data. It lives in a SQLite DB (WAL mode), a few JSON settings files, and
a recordings folder whose location the user can move. `meetings.folder_path` is an absolute path,
so a plain copy breaks every audio link on a Mac with another username or folder. Models and
Keychain secrets are tied to the machine. The owner wants to move to a new Mac, or recover from a
dead one, without running Time Machine (specs/0092).

## Decision

A backup is one plain `.zip` the user keeps. Format 1:

```
manifest.json                       format, app version, schema_version, counts, includes_audio
db/meeting_minutes.sqlite           VACUUM INTO snapshot, FTS tables rebuilt
settings/<file>.json                allowlisted files only
recordings/<meeting-folder>/...     media files only when audio is included
```

- **Allowlist, not denylist.** Only named settings files travel. A file added to the app data
  directory later stays home until someone adds it on purpose.
- **Snapshot with `VACUUM INTO`**, never a raw copy of a live WAL DB. VACUUM may renumber the
  implicit rowids the external-content FTS5 tables key on, so every FTS table is rebuilt in the
  snapshot before zipping.
- **Machine-bound tables are not restored.** `settings`, `transcript_settings`,
  `summary_prompts_meta`, `licensing` and `google_calendar_*` keep this Mac's rows in both import
  modes (`backup::MACHINE_BOUND_TABLES`). Models, Keychain secrets, agent access registration and
  the recordings location also stay with the Mac.
- **Schema-version refusal rule.** `manifest.schema_version` is the newest applied migration.
  A backup newer than the running build is refused with "update Nixon, then import it". An older
  backup is migrated on a staged copy first. Nothing downgrades.
- **Every table is classified** (merged, or machine-bound) and a test enumerates `sqlite_master`,
  so a future migration cannot silently skip or mis-merge a table.
- **Import is staged.** Replace extracts to a staging directory, writes a pending marker and
  restarts; the swap happens at launch before the DB opens, so no writer races it, and a failure
  puts the old DB back. Add attaches the staged DB and runs `INSERT OR IGNORE` by ID with explicit
  foreign-key filters; only meetings new to this Mac bring their child rows.
- **Why plain zip, unencrypted.** The on-machine data is not encrypted either, a zip opens with
  anything, and a person can recover their recordings without Nixon. The cost: whoever holds the
  file can read it, and a synced folder stores it with that service. The app says so.

## Consequences

- The format is versioned (`format: 1`); a different layout bumps it and old readers refuse it.
- Replacing never touches recording folders already on disk. Pruning only deletes automatic
  backups (`Nixon Auto Backup YYYY-MM-DD HHMM.zip`, newest five kept); a backup made by hand
  (`Nixon Backup …`) stays until the user deletes it from the list, which only accepts a backup
  file directly inside the backup folder.
- Adding a setting that should travel means adding it to the allowlist and, if it is a new table,
  classifying it. Both are visible in review.
