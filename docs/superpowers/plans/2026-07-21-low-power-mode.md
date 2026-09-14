# Low Power Mode + Attendee-Removal Fix — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Battery-triggered deferral of transcription/diarization/summary (record audio now, process on AC after a prompt), a per-meeting live/defer override that works mid-recording, and a tombstone fix so attendee removals made while recording survive the calendar re-seed.

**Architecture:** Reuse the existing record-only path (`live_transcription_enabled = false` branch) as the deferral mechanism; add an IOKit power monitor, a pure effective-mode decision, a `meetings.processing_mode` override column, a global atomic that lets the running pipeline attach/detach its VAD/STT stage mid-recording, and a frontend backlog prompt that drives the existing retranscription → diarization → summary machinery sequentially. The attendee fix converts the roster `DELETE` into a `removed_at` soft-delete that the calendar seed's `INSERT OR IGNORE` naturally respects.

**Tech Stack:** Tauri 2 (Rust backend, `frontend/src-tauri/`), Next.js 14 / React 18 (`frontend/src/`), sqlx + SQLite (forward-only migrations), tauri-plugin-store, Vitest, cargo test.

**Spec:** `docs/superpowers/specs/2026-07-21-low-power-mode-design.md` — read it first.

## Global Constraints

- All Rust paths below are relative to `frontend/src-tauri/`, all frontend paths relative to `frontend/`, unless they start with `docs/` or `scripts/`.
- **File-size ratchet (`scripts/check-file-size.sh`)**: no production `.rs/.ts/.tsx` file over 800 lines unless allowlisted in `scripts/file-size-allowlist.txt`; **allowlisted files may only shrink**. Shrink-only files this plan touches: `src/audio/pipeline.rs` (2007), `src/audio/recording_commands.rs` (1558), `src/diarization/commands.rs` (1481). Every task that edits one of these pairs its addition with an offsetting extraction and ends by running `scripts/check-file-size.sh`. `src/database/repositories/meeting_participant.rs` is 791/800 — its new tests go in `frontend/src-tauri/tests/` (exempt), not inline.
- **New Tauri commands register in `src/registry.rs`** (never in `lib.rs`) — append to the matching subsystem section.
- Rust conventions: `anyhow::Result` internally, `Result<_, String>` at the command layer; command (frontend→Rust) + event (Rust→frontend) pattern.
- Migrations are forward-only, named `YYYYMMDDHHMMSS_description.sql`, with a comment header explaining semantics (mirror `migrations/20260703000001_add_meeting_template.sql`).
- Rust gate: `cd frontend/src-tauri && source ~/.cargo/env && cargo check && cargo clippy && cargo test` (needs the llama-helper sidecar present; if missing, build once via `cd frontend && ./dev-vinyl.sh` — see CLAUDE.md — or run `cargo test --lib` for repo-level tests). Frontend gate: `cd frontend && pnpm lint && pnpm test`.
- Known pre-existing failure: the `vad_filter` adversarial-fixtures integration test fails on macOS 15.x. Not a regression; ignore it, never "fix" it.
- Commit after every task with a `feat:`/`fix:`/`refactor:` message.

---

## Part A — Attendee-removal tombstone fix

### Task 1: `removed_at` tombstone in the roster repository

**Files:**
- Create: `migrations/20260721000000_add_participant_removed_at.sql`
- Modify: `src/database/repositories/meeting_participant.rs`
- Test: `frontend/src-tauri/tests/participant_tombstone.rs` (new integration test — exempt from the ratchet)

**Interfaces:**
- Consumes: existing `MeetingParticipantsRepository` (`add`, `remove`, `list`, `count_for_meeting`, `remote_person_ids`, `seed_from_attendees`, `add_identified`, `attendee_previews`), `PeopleRepository::create`, `Attendee`.
- Produces: `remove` becomes a soft-delete (same signature `remove(pool, meeting_id, person_id) -> Result<bool, SqlxError>`); new `restore_or_add(pool: &SqlitePool, meeting_id: &str, person_id: &str, source: &str) -> Result<bool, SqlxError>` (true = row inserted OR tombstone cleared); all read/count queries exclude tombstoned rows. Task 2 depends on `restore_or_add`.

- [ ] **Step 1: Write the failing integration tests**

Create `frontend/src-tauri/tests/participant_tombstone.rs`. Look at an existing file in `frontend/src-tauri/tests/` (e.g. `db_lifecycle.rs`) for the crate import name and the in-memory-pool-through-migrations pattern (`sqlx::migrate!` is not available from an integration test — instead copy the `pool_with_schema()` helper style used there; if the existing tests build the pool via the crate's public test helpers, mirror that). The tests, written against the real migration set:

```rust
//! Tombstone semantics for meeting_participants.removed_at:
//! a mid-recording removal must survive the calendar seed-on-view.

use sqlx::SqlitePool;
// Adjust this `use` to the crate name used by the existing files in tests/.
use vinyl::calendar::eventkit::Attendee;
use vinyl::database::repositories::meeting_participant::MeetingParticipantsRepository;
use vinyl::database::repositories::people::PeopleRepository;

async fn pool_with_schema() -> SqlitePool {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::migrate!("./migrations").run(&pool).await.unwrap();
    pool
}

fn attendee(name: &str, email: Option<&str>) -> Attendee {
    Attendee {
        name: name.to_string(),
        email: email.map(str::to_string),
        is_current_user: false,
        is_distribution_list: false,
        photo_data_uri: None,
    }
}

#[tokio::test]
async fn seed_does_not_resurrect_removed_calendar_attendee() {
    let pool = pool_with_schema().await;
    let attendees = vec![attendee("Alice", Some("alice@example.com"))];

    MeetingParticipantsRepository::seed_from_attendees(&pool, "m1", &attendees)
        .await
        .unwrap();
    let roster = MeetingParticipantsRepository::list(&pool, "m1").await.unwrap();
    assert_eq!(roster.len(), 1);
    let alice_id = roster[0].person_id.clone();

    assert!(MeetingParticipantsRepository::remove(&pool, "m1", &alice_id)
        .await
        .unwrap());
    assert!(MeetingParticipantsRepository::list(&pool, "m1")
        .await
        .unwrap()
        .is_empty());

    // The bug: this re-seed used to resurrect Alice.
    let added = MeetingParticipantsRepository::seed_from_attendees(&pool, "m1", &attendees)
        .await
        .unwrap();
    assert_eq!(added, 0, "seed must respect the tombstone");
    assert!(MeetingParticipantsRepository::list(&pool, "m1")
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn seed_does_not_resurrect_removed_emailless_attendee() {
    let pool = pool_with_schema().await;
    let attendees = vec![attendee("Bob", None)]; // no email → dedup is by name snapshot

    MeetingParticipantsRepository::seed_from_attendees(&pool, "m1", &attendees)
        .await
        .unwrap();
    let bob_id = MeetingParticipantsRepository::list(&pool, "m1").await.unwrap()[0]
        .person_id
        .clone();
    MeetingParticipantsRepository::remove(&pool, "m1", &bob_id)
        .await
        .unwrap();

    let added = MeetingParticipantsRepository::seed_from_attendees(&pool, "m1", &attendees)
        .await
        .unwrap();
    assert_eq!(added, 0, "name-dedup snapshot must include tombstoned rows");
    let people: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM people WHERE display_name = 'Bob'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(people.0, 1, "no duplicate name-only person minted");
}

#[tokio::test]
async fn restore_or_add_clears_tombstone_and_counts_exclude_removed() {
    let pool = pool_with_schema().await;
    let alice = PeopleRepository::create(&pool, "Alice", Some("a@x.com"), None, None)
        .await
        .unwrap();
    let bob = PeopleRepository::create(&pool, "Bob", Some("b@x.com"), None, None)
        .await
        .unwrap();
    MeetingParticipantsRepository::add(&pool, "m1", &alice.id, "calendar")
        .await
        .unwrap();
    MeetingParticipantsRepository::add(&pool, "m1", &bob.id, "calendar")
        .await
        .unwrap();

    MeetingParticipantsRepository::remove(&pool, "m1", &alice.id)
        .await
        .unwrap();
    // The diarization speaker cap consumers must not see removed rows.
    assert_eq!(
        MeetingParticipantsRepository::count_for_meeting(&pool, "m1").await.unwrap(),
        1
    );
    assert_eq!(
        MeetingParticipantsRepository::remote_person_ids(&pool, "m1").await.unwrap(),
        vec![bob.id.clone()]
    );

    // Manual re-add restores the same row (clears removed_at) rather than no-opping.
    assert!(
        MeetingParticipantsRepository::restore_or_add(&pool, "m1", &alice.id, "manual")
            .await
            .unwrap()
    );
    assert_eq!(
        MeetingParticipantsRepository::count_for_meeting(&pool, "m1").await.unwrap(),
        2
    );
    // add() (the seed path) on an ACTIVE row is still a no-op.
    assert!(!MeetingParticipantsRepository::add(&pool, "m1", &alice.id, "calendar")
        .await
        .unwrap());
}

#[tokio::test]
async fn add_identified_restores_removed_participant() {
    let pool = pool_with_schema().await;
    let alice = PeopleRepository::create(&pool, "Alice", Some("a@x.com"), None, None)
        .await
        .unwrap();
    MeetingParticipantsRepository::add(&pool, "m1", &alice.id, "calendar")
        .await
        .unwrap();
    MeetingParticipantsRepository::remove(&pool, "m1", &alice.id)
        .await
        .unwrap();

    // If diarization identifies her as a speaker, she demonstrably attended → restore.
    assert!(MeetingParticipantsRepository::add_identified(&pool, "m1", &alice.id)
        .await
        .unwrap());
    let roster = MeetingParticipantsRepository::list(&pool, "m1").await.unwrap();
    assert_eq!(roster.len(), 1);
    assert_eq!(roster[0].source, "identified");
}
```

Note: `pool_with_schema` requires `sqlx::migrate!("./migrations")` to resolve — from `tests/` the macro path is relative to the crate root (`frontend/src-tauri/`), so `"./migrations"` is correct. If the crate is not importable as `vinyl`, check `Cargo.toml` `[lib] name` / how `tests/db_lifecycle.rs` imports it and adjust.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd frontend/src-tauri && source ~/.cargo/env && cargo test --features metal --test participant_tombstone`
Expected: FAIL — `restore_or_add` not found (compile error), and after stubbing, `seed_does_not_resurrect_removed_calendar_attendee` fails at `assert_eq!(added, 0)`.

- [ ] **Step 3: Write the migration**

`migrations/20260721000000_add_participant_removed_at.sql`:

```sql
-- Attendee-removal tombstone (specs 2026-07-21 low-power-mode design §6): a user's
-- roster removal must survive the calendar seed-on-view (`api_get_meeting_participants`
-- re-seeds on every read). NULL — the value for all existing rows — means the
-- participant is active. Non-NULL records WHEN the user removed them; the row is kept
-- so `seed_from_attendees`' INSERT OR IGNORE can never resurrect it. Cleared back to
-- NULL when the participant is re-added manually or identified as a speaker.
ALTER TABLE meeting_participants ADD COLUMN removed_at TEXT;
```

- [ ] **Step 4: Implement the repository changes**

In `src/database/repositories/meeting_participant.rs`:

(a) `remove` (currently the `DELETE` at line ~196) becomes a soft-delete:

```rust
    /// Remove a participant from a meeting's roster — a SOFT delete: stamps
    /// `removed_at` and keeps the row as a tombstone so the calendar
    /// seed-on-view (`seed_from_attendees`, INSERT OR IGNORE) can never
    /// resurrect the participant. The Person and any `speakers.person_id`
    /// link survive. Returns whether an active row was tombstoned.
    pub async fn remove(
        pool: &SqlitePool,
        meeting_id: &str,
        person_id: &str,
    ) -> Result<bool, SqlxError> {
        let now = Utc::now().to_rfc3339();
        let res = sqlx::query(
            "UPDATE meeting_participants SET removed_at = ?
             WHERE meeting_id = ? AND person_id = ? AND removed_at IS NULL",
        )
        .bind(&now)
        .bind(meeting_id)
        .bind(person_id)
        .execute(pool)
        .await?;
        Ok(res.rows_affected() > 0)
    }
```

(b) New `restore_or_add`, placed right after `add`:

```rust
    /// Add for the MANUAL/IDENTIFIED paths: insert the row, or — when a
    /// tombstoned row exists — clear `removed_at` (the user or diarization is
    /// explicitly putting this person back). The calendar seed keeps using
    /// [`add`](Self::add), whose INSERT OR IGNORE respects tombstones.
    pub async fn restore_or_add(
        pool: &SqlitePool,
        meeting_id: &str,
        person_id: &str,
        source: &str,
    ) -> Result<bool, SqlxError> {
        if Self::add(pool, meeting_id, person_id, source).await? {
            return Ok(true);
        }
        let res = sqlx::query(
            "UPDATE meeting_participants SET removed_at = NULL, source = ?
             WHERE meeting_id = ? AND person_id = ? AND removed_at IS NOT NULL",
        )
        .bind(source)
        .bind(meeting_id)
        .bind(person_id)
        .execute(pool)
        .await?;
        Ok(res.rows_affected() > 0)
    }
```

(c) `add_identified` calls `restore_or_add(pool, meeting_id, person_id, "identified")` instead of `add`.

(d) Filter tombstones from every read: add `AND mp.removed_at IS NULL` to the `WHERE` of `list` (line ~141) and, in `attendee_previews`' CTE, to the `FROM meeting_participants mp JOIN people p ...` (line ~109–110: `JOIN people p ON p.id = mp.person_id WHERE mp.removed_at IS NULL`); add `AND removed_at IS NULL` to `count_for_meeting` (line ~213) and `remote_person_ids` (line ~229–230).

(e) `seed_from_attendees`' email-less name-dedup snapshot must include tombstoned rows (otherwise a removed email-less attendee gets a fresh person minted and re-added). Replace `let existing = Self::list(pool, meeting_id).await?;` + the `existing_names` mapping (lines ~264–268) with a direct query:

```rust
        // Snapshot existing names INCLUDING tombstoned rows so re-seeding an
        // email-less invitee the user removed is a no-op, not a resurrection.
        let existing_names: std::collections::HashSet<String> = sqlx::query_as::<_, (String,)>(
            "SELECT p.display_name FROM meeting_participants mp
             JOIN people p ON p.id = mp.person_id WHERE mp.meeting_id = ?",
        )
        .bind(meeting_id)
        .fetch_all(pool)
        .await?
        .into_iter()
        .map(|(name,)| name.trim().to_lowercase())
        .collect();
```

(f) The inline `#[cfg(test)] mod tests` `test_pool()` (line ~360) creates `meeting_participants` by hand — add `removed_at TEXT` to that CREATE TABLE so inline tests match the migrated schema.

(g) Update the module doc comment (lines 20–25): note that `remove` is now a tombstone and why. Keep the file ≤ 800 lines (it starts at 791 and these edits are roughly +45/−10; if `wc -l` exceeds 800, move the two largest inline tests — `attendee_previews_are_bounded_and_owner_inclusive` and `list_joins_cached_photo_by_normalized_email` — into `tests/participant_tombstone.rs` unchanged apart from imports).

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cd frontend/src-tauri && cargo test --features metal --test participant_tombstone && cargo test --features metal --lib database::repositories::meeting_participant`
Expected: all PASS (inline repo tests included — `remove_leaves_person_intact` still passes because `list` filters the tombstone).

Run: `wc -l src/database/repositories/meeting_participant.rs` — expected ≤ 800. Then `cd ../.. && scripts/check-file-size.sh` — expected PASS.

- [ ] **Step 6: Commit**

```bash
git add frontend/src-tauri/migrations/20260721000000_add_participant_removed_at.sql \
        frontend/src-tauri/src/database/repositories/meeting_participant.rs \
        frontend/src-tauri/tests/participant_tombstone.rs
git commit -m "fix: tombstone attendee removals so the calendar seed cannot resurrect them"
```

### Task 2: Command layer uses restore semantics

**Files:**
- Modify: `src/diarization/commands.rs` (shrink-only, 1481 cap — this change is net 0 lines)

**Interfaces:**
- Consumes: `MeetingParticipantsRepository::restore_or_add` (Task 1).
- Produces: `api_add_meeting_participant` restores tombstoned participants. No signature changes.

- [ ] **Step 1: Switch the manual-add path**

In `api_add_meeting_participant` (line ~1244), change:

```rust
    MeetingParticipantsRepository::add(&pool, &meeting_id, &resolved_person_id, "manual")
```
to
```rust
    MeetingParticipantsRepository::restore_or_add(&pool, &meeting_id, &resolved_person_id, "manual")
```

Also check `git grep -n "MeetingParticipantsRepository::add(" -- frontend/src-tauri/src` for any other MANUAL/IDENTIFIED call sites (the calendar seed and `add_identified` internals are already correct from Task 1); switch any user-intent add to `restore_or_add`.

- [ ] **Step 2: Verify**

Run: `cd frontend/src-tauri && cargo check && cargo clippy && cargo test --features metal --lib && cd ../.. && scripts/check-file-size.sh`
Expected: clean; `diarization/commands.rs` did not grow (`wc -l` ≤ 1481).

- [ ] **Step 3: Commit**

```bash
git add frontend/src-tauri/src/diarization/commands.rs
git commit -m "fix: manual attendee re-add clears the removal tombstone"
```

No frontend changes are needed for the bug: `ParticipantsPanel.tsx`'s optimistic remove + the now-consistent reads mean removals persist across the meeting-details remount, and the diarization cap queries (`count_for_meeting`, `remote_person_ids`) now honor removals automatically.

---

## Part B — Low Power Mode

### Task 3: Power-source monitor (`power` module)

**Files:**
- Create: `src/power/mod.rs`, `src/power/macos.rs`
- Modify: `src/lib.rs` (add `pub mod power;` to the module list; one `spawn` call in setup), `src/registry.rs` (register command)

**Interfaces:**
- Produces:
  - `power::PowerSource` enum (`Ac | Battery | Unknown`), serialized lowercase.
  - `power::current_power_source() -> PowerSource` — fresh IOKit query.
  - `power::is_on_battery() -> bool` — convenience over the fresh query.
  - `power::spawn_power_monitor(app: AppHandle)` — IOKit run-loop notification thread; emits Tauri event `power-source-changed` with payload `{ "onBattery": bool }` on every change.
  - Tauri command `api_get_power_state() -> Result<serde_json::Value, String>` returning `{ "onBattery": bool }`.
- Consumed by: Task 5 (recording start), Task 9 (frontend debounce/prompt).

- [ ] **Step 1: Write `src/power/mod.rs`**

```rust
//! Power-source awareness (low-power-mode spec §1): battery vs AC, macOS-only.
//!
//! Two consumers:
//! - the recording start path queries [`is_on_battery`] fresh to pick the
//!   effective processing mode;
//! - the frontend backlog prompt listens for the `power-source-changed` event
//!   emitted by the notification thread spawned in [`spawn_power_monitor`].
//!
//! Event-driven via `IOPSNotificationCreateRunLoopSource` — no polling. On
//! non-macOS builds everything degrades to `Unknown` / never-on-battery.

#[cfg(target_os = "macos")]
mod macos;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Runtime};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PowerSource {
    Ac,
    Battery,
    Unknown,
}

/// Fresh query of the providing power source.
pub fn current_power_source() -> PowerSource {
    #[cfg(target_os = "macos")]
    {
        macos::query_power_source()
    }
    #[cfg(not(target_os = "macos"))]
    {
        PowerSource::Unknown
    }
}

/// `true` only when we positively know we're on battery — `Unknown` counts as
/// AC so a query failure can never silently degrade a meeting to deferred mode.
pub fn is_on_battery() -> bool {
    current_power_source() == PowerSource::Battery
}

/// Spawn the change-notification thread (call once at app setup). Each IOKit
/// power-source change re-queries and emits `power-source-changed`.
pub fn spawn_power_monitor<R: Runtime>(app: AppHandle<R>) {
    #[cfg(target_os = "macos")]
    {
        macos::spawn_notification_thread(move || {
            let on_battery = is_on_battery();
            log::info!("power source changed: on_battery={on_battery}");
            let _ = app.emit(
                "power-source-changed",
                serde_json::json!({ "onBattery": on_battery }),
            );
        });
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = app;
    }
}

/// Frontend query: current power state (used at mount and before prompting).
#[tauri::command]
pub async fn api_get_power_state() -> Result<serde_json::Value, String> {
    Ok(serde_json::json!({ "onBattery": is_on_battery() }))
}
```

- [ ] **Step 2: Write `src/power/macos.rs`**

```rust
//! IOKit power-source FFI (macOS). Kept thin and unsafe-contained; the
//! decision logic lives in `mod.rs` / `audio::processing_mode` where it is
//! testable without hardware.

use super::PowerSource;
use std::ffi::{c_char, c_void, CStr};

type CFTypeRef = *const c_void;
type CFStringRef = *const c_void;
type CFRunLoopSourceRef = *const c_void;
type CFRunLoopRef = *const c_void;
type CFIndex = isize;

const K_CF_STRING_ENCODING_UTF8: u32 = 0x0800_0100;

#[link(name = "IOKit", kind = "framework")]
extern "C" {
    fn IOPSCopyPowerSourcesInfo() -> CFTypeRef;
    fn IOPSGetProvidingPowerSourceType(snapshot: CFTypeRef) -> CFStringRef;
    fn IOPSNotificationCreateRunLoopSource(
        callback: extern "C" fn(context: *mut c_void),
        context: *mut c_void,
    ) -> CFRunLoopSourceRef;
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFRunLoopGetCurrent() -> CFRunLoopRef;
    fn CFRunLoopAddSource(rl: CFRunLoopRef, source: CFRunLoopSourceRef, mode: CFStringRef);
    fn CFRunLoopRun();
    fn CFRelease(cf: CFTypeRef);
    fn CFStringGetCString(
        s: CFStringRef,
        buffer: *mut c_char,
        size: CFIndex,
        encoding: u32,
    ) -> u8;
    static kCFRunLoopDefaultMode: CFStringRef;
}

/// Query the providing power source ("AC Power" / "Battery Power" / "UPS Power").
pub(super) fn query_power_source() -> PowerSource {
    unsafe {
        let snapshot = IOPSCopyPowerSourcesInfo();
        if snapshot.is_null() {
            return PowerSource::Unknown;
        }
        // Get-rule: the returned CFString is NOT owned by us; only the
        // snapshot needs releasing.
        let source_type = IOPSGetProvidingPowerSourceType(snapshot);
        let result = if source_type.is_null() {
            PowerSource::Unknown
        } else {
            let mut buf = [0 as c_char; 64];
            if CFStringGetCString(source_type, buf.as_mut_ptr(), 64, K_CF_STRING_ENCODING_UTF8)
                != 0
            {
                match CStr::from_ptr(buf.as_ptr()).to_string_lossy().as_ref() {
                    "Battery Power" => PowerSource::Battery,
                    "AC Power" | "UPS Power" => PowerSource::Ac,
                    _ => PowerSource::Unknown,
                }
            } else {
                PowerSource::Unknown
            }
        };
        CFRelease(snapshot);
        result
    }
}

/// Park a dedicated thread in a CFRunLoop that fires `on_change` on every
/// power-source notification. The closure is intentionally leaked — the
/// monitor lives for the whole process.
pub(super) fn spawn_notification_thread(on_change: impl Fn() + Send + 'static) {
    extern "C" fn trampoline(context: *mut c_void) {
        let cb = unsafe { &*(context as *const Box<dyn Fn() + Send>) };
        cb();
    }
    let boxed: Box<Box<dyn Fn() + Send>> = Box::new(Box::new(on_change));
    // Pass the pointer as usize so the spawned closure is Send.
    let context = Box::into_raw(boxed) as usize;
    if let Err(e) = std::thread::Builder::new()
        .name("power-monitor".into())
        .spawn(move || unsafe {
            let source = IOPSNotificationCreateRunLoopSource(trampoline, context as *mut c_void);
            if source.is_null() {
                log::warn!("power monitor: IOPSNotificationCreateRunLoopSource failed; low-power auto-detection disabled");
                return;
            }
            CFRunLoopAddSource(CFRunLoopGetCurrent(), source, kCFRunLoopDefaultMode);
            CFRunLoopRun();
        })
    {
        log::warn!("power monitor thread failed to spawn: {e}");
    }
}
```

- [ ] **Step 3: Wire it up**

In `src/lib.rs`: add `pub mod power;` alongside the other top-level module declarations, and in the setup closure directly after `aggregation::prep_jobs::spawn_prep_generator(_app.handle().clone());` (line ~335) add:

```rust
            // Power-source monitor (low-power-mode spec §1): emits
            // `power-source-changed` so the frontend can prompt for deferred
            // backlog processing when back on AC.
            power::spawn_power_monitor(_app.handle().clone());
```

In `src/registry.rs`: add a new section near the audio commands:

```rust
        // Power state (low-power mode)
        power::api_get_power_state,
```
and add `power` to the `use crate::{...}` list.

- [ ] **Step 4: Verify**

Run: `cd frontend/src-tauri && cargo check && cargo clippy`
Expected: clean. (No unit test — this is thin FFI; behavior is exercised manually in Task 11's smoke test.)

- [ ] **Step 5: Commit**

```bash
git add frontend/src-tauri/src/power/ frontend/src-tauri/src/lib.rs frontend/src-tauri/src/registry.rs
git commit -m "feat: IOKit power-source monitor with power-source-changed event"
```

### Task 4: Low-power preference, effective-mode decision, per-meeting override column

**Files:**
- Create: `src/audio/processing_mode.rs`, `migrations/20260721000001_add_meeting_processing_mode.sql`
- Modify: `src/audio/recording_preferences.rs`, `src/audio/mod.rs` (declare `pub mod processing_mode;`), `src/database/models.rs`, `src/database/repositories/meeting/crud.rs`, `src/meetings/` (commands — find the module file where meeting-metadata commands like `update_meeting_title`'s command wrapper live via `git grep -n "api_" frontend/src-tauri/src/meetings/`), `src/registry.rs`
- Test: inline `#[cfg(test)]` in `processing_mode.rs`; repo tests inline in `crud.rs`'s existing test mod (check its size first — if near 800, put them in `frontend/src-tauri/tests/participant_tombstone.rs`'s style as a new `tests/processing_mode.rs`)

**Interfaces:**
- Produces:
  - `RecordingPreferences.low_power_on_battery: bool` (serde default **true**).
  - `audio::processing_mode::effective_live_stt(live_transcription_enabled: bool, low_power_on_battery: bool, on_battery: bool, meeting_override: Option<&str>) -> bool`.
  - `meetings.processing_mode TEXT` column (`NULL` | `'live'` | `'defer'`), `MeetingModel.processing_mode: Option<String>`.
  - `MeetingsRepository::get_processing_mode(pool, meeting_id) -> Result<Option<String>, SqlxError>` and `set_processing_mode(pool, meeting_id, mode: Option<&str>) -> Result<bool, SqlxError>` (mirror `get_meeting_template`/`set_meeting_template`, `crud.rs:343–387`, including blank-string normalization; additionally reject values other than `live`/`defer` with `SqlxError::Protocol`).
  - Tauri commands `api_get_meeting_processing_mode(meeting_id) -> Result<Option<String>, String>`, `api_set_meeting_processing_mode(meeting_id, mode: Option<String>) -> Result<(), String>`.
- Consumed by: Tasks 5, 6, 7, 9, 10.

- [ ] **Step 1: Write the failing unit tests for the decision** (in `src/audio/processing_mode.rs`, written together with the fn below — run tests before implementing the body by stubbing `todo!()` if you want the strict red step, or accept compile-fail as the red)

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn override_wins_in_both_directions() {
        // 'live' forces live even on battery with low-power on…
        assert!(effective_live_stt(true, true, true, Some("live")));
        // …and even when the global record-only preference is set.
        assert!(effective_live_stt(false, true, false, Some("live")));
        // 'defer' forces deferral even on AC.
        assert!(!effective_live_stt(true, true, false, Some("defer")));
    }

    #[test]
    fn low_power_defers_only_on_battery() {
        assert!(!effective_live_stt(true, true, true, None), "battery + switch on → defer");
        assert!(effective_live_stt(true, true, false, None), "AC → live");
        assert!(effective_live_stt(true, false, true, None), "switch off → live on battery");
    }

    #[test]
    fn record_only_preference_still_defers() {
        assert!(!effective_live_stt(false, false, false, None));
    }

    #[test]
    fn unknown_override_falls_back_to_global() {
        assert!(effective_live_stt(true, true, false, Some("bogus")));
    }
}
```

- [ ] **Step 2: Implement `src/audio/processing_mode.rs`**

```rust
//! Effective processing-mode decision (low-power-mode spec §§2–3).
//!
//! Pure so it is unit-testable: the caller supplies the global preferences,
//! the live power state, and the meeting's `processing_mode` override.

/// `meetings.processing_mode` value forcing full live processing.
pub const MODE_LIVE: &str = "live";
/// `meetings.processing_mode` value forcing deferral (record-only).
pub const MODE_DEFER: &str = "defer";

/// Should this recording session run live VAD/STT?
///
/// Precedence: per-meeting override → low-power-on-battery → the global
/// `live_transcription_enabled` preference (specs/0029 WS7.2 record-only mode).
pub fn effective_live_stt(
    live_transcription_enabled: bool,
    low_power_on_battery: bool,
    on_battery: bool,
    meeting_override: Option<&str>,
) -> bool {
    match meeting_override {
        Some(MODE_LIVE) => true,
        Some(MODE_DEFER) => false,
        _ => live_transcription_enabled && !(low_power_on_battery && on_battery),
    }
}
```

Declare `pub mod processing_mode;` in `src/audio/mod.rs`.

- [ ] **Step 3: Preference field**

In `src/audio/recording_preferences.rs`, add to `RecordingPreferences` (after `live_transcription_enabled`):

```rust
    /// Low Power Mode (2026-07-21 spec §2): when `true` (the default) and the
    /// Mac is on battery, recordings run in record-only mode — audio saved,
    /// VAD/STT and summaries deferred until back on AC — unless the meeting's
    /// `processing_mode` override says otherwise. `#[serde(default)]` keeps
    /// previously-stored preferences deserializing (as `true`).
    #[serde(default = "default_low_power_on_battery")]
    pub low_power_on_battery: bool,
```

with `fn default_low_power_on_battery() -> bool { true }` next to the existing default fn, and `low_power_on_battery: true,` in `impl Default`.

- [ ] **Step 4: Migration + model + repo accessors + commands**

`migrations/20260721000001_add_meeting_processing_mode.sql`:

```sql
-- Per-meeting processing-mode override (low-power-mode spec §3). NULL — the value
-- for all existing and new rows — means "follow the global low-power/battery
-- decision". 'live' forces full live processing (set when the user flips a
-- deferred meeting live, incl. mid-recording); 'defer' forces record-only and
-- marks the meeting pending for backlog processing. Cleared back to NULL when
-- backlog/stop-time processing completes.
ALTER TABLE meetings ADD COLUMN processing_mode TEXT;
```

`src/database/models.rs` — add to `MeetingModel` after `template_id` (same attribute pattern):

```rust
    /// Per-meeting processing-mode override (low-power-mode spec §3):
    /// NULL = follow global, 'live' = force full processing, 'defer' =
    /// force deferral / pending backlog. `#[sqlx(default)]` so legacy
    /// explicit-column SELECTs still decode.
    #[serde(default)]
    #[sqlx(default)]
    pub processing_mode: Option<String>,
```

`src/database/repositories/meeting/crud.rs` — add `processing_mode` to the explicit column list in `get_meeting_metadata`'s SELECT (line ~308), and add accessors mirroring the template pair (place directly after `set_meeting_template`):

```rust
    /// Reads the per-meeting processing-mode override (low-power-mode spec §3).
    /// NULL/absent → None → "follow the global decision".
    pub async fn get_processing_mode(
        pool: &SqlitePool,
        meeting_id: &str,
    ) -> Result<Option<String>, SqlxError> {
        if meeting_id.trim().is_empty() {
            return Err(SqlxError::Protocol("meeting_id cannot be empty".to_string()));
        }
        let row: Option<(Option<String>,)> =
            sqlx::query_as("SELECT processing_mode FROM meetings WHERE id = ?")
                .bind(meeting_id)
                .fetch_optional(pool)
                .await?;
        Ok(row.and_then(|(mode,)| mode))
    }

    /// Persists the processing-mode override. `None`/blank clears to NULL.
    /// Only 'live' and 'defer' are accepted. Returns whether a row was updated
    /// (false ⇒ meeting not found). Does not bump `updated_at` (metadata, not
    /// content — mirrors `set_meeting_template`).
    pub async fn set_processing_mode(
        pool: &SqlitePool,
        meeting_id: &str,
        mode: Option<&str>,
    ) -> Result<bool, SqlxError> {
        if meeting_id.trim().is_empty() {
            return Err(SqlxError::Protocol("meeting_id cannot be empty".to_string()));
        }
        let mode = mode.map(str::trim).filter(|m| !m.is_empty());
        if let Some(m) = mode {
            if m != crate::audio::processing_mode::MODE_LIVE
                && m != crate::audio::processing_mode::MODE_DEFER
            {
                return Err(SqlxError::Protocol(format!(
                    "invalid processing_mode {m:?} (expected 'live' or 'defer')"
                )));
            }
        }
        let result = sqlx::query("UPDATE meetings SET processing_mode = ? WHERE id = ?")
            .bind(mode)
            .bind(meeting_id)
            .execute(pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }
```

Commands — in the meetings command module found via the grep above (same file that wraps other meeting-metadata commands; if none fits, create `src/meetings/processing_mode_commands.rs` and declare it in `src/meetings/mod.rs`):

```rust
/// Per-meeting processing-mode override (low-power-mode spec §3).
#[tauri::command]
pub async fn api_get_meeting_processing_mode<R: Runtime>(
    app: AppHandle<R>,
    meeting_id: String,
) -> Result<Option<String>, String> {
    let state = app.state::<AppState>();
    let pool = state.db_manager.pool();
    MeetingsRepository::get_processing_mode(pool, &meeting_id)
        .await
        .map_err(|e| format!("Failed to load processing mode: {e}"))
}

/// Persist the override. Pass `mode: None` to clear (follow global).
#[tauri::command]
pub async fn api_set_meeting_processing_mode<R: Runtime>(
    app: AppHandle<R>,
    meeting_id: String,
    mode: Option<String>,
) -> Result<(), String> {
    let state = app.state::<AppState>();
    let pool = state.db_manager.pool();
    let updated = MeetingsRepository::set_processing_mode(pool, &meeting_id, mode.as_deref())
        .await
        .map_err(|e| format!("Failed to save processing mode: {e}"))?;
    if !updated {
        return Err("Meeting not found".to_string());
    }
    Ok(())
}
```

Register both in `src/registry.rs` under the meetings section.

Repo tests (in `crud.rs`'s existing `#[cfg(test)]` mod if the file stays ≤ 800 lines, else a new `frontend/src-tauri/tests/processing_mode.rs`): set/get round-trip, `None` clears, invalid value rejected, unknown meeting returns `Ok(false)` from the setter.

- [ ] **Step 5: Verify**

Run: `cd frontend/src-tauri && cargo test --features metal --lib audio::processing_mode && cargo test --features metal --lib database && cargo clippy && cd ../.. && scripts/check-file-size.sh`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add -A frontend/src-tauri/migrations frontend/src-tauri/src docs
git commit -m "feat: low-power preference, effective-mode decision, per-meeting processing_mode"
```

### Task 5: Recording start honors the effective mode

**Files:**
- Create: `src/audio/live_toggle.rs` (statics + start-time helper; the mid-meeting command arrives in Task 6)
- Modify: `src/audio/mod.rs` (declare module), `src/audio/recording_commands.rs` (shrink-only 1558 cap — offset below)

**Interfaces:**
- Consumes: `power::is_on_battery`, `processing_mode::effective_live_stt`, `MeetingsRepository::get_processing_mode`.
- Produces:
  - `live_toggle::decide_session_mode(app, prefs_live, prefs_low_power, meeting_id: Option<&str>) -> bool` — computes the effective live flag, logs, and emits `processing-mode-changed` `{ "meetingId": ..., "liveTranscription": bool, "onBattery": bool }`.
  - `live_toggle::stash_receiver(receiver)` / `live_toggle::take_receiver()` for the pending `mpsc::Receiver<TranscriptionChunk>`.
  - `live_toggle::set_session_flag(Arc<AtomicBool>)` / `live_toggle::session_flag() -> Option<Arc<AtomicBool>>` / `live_toggle::clear_session()` (flag + receiver dropped at stop).
  - Event `processing-mode-changed` (also emitted by Task 6's command).
- Consumed by: Task 6 (pipeline + toggle command), frontend Tasks 9–10.

- [ ] **Step 1: Write `src/audio/live_toggle.rs`**

```rust
//! Session-scoped live-STT toggle state (low-power-mode spec §§3–4).
//!
//! Bridges three parties without growing the (ratchet-frozen) recording
//! commands file: the start path decides the initial mode and stashes the
//! transcription receiver when deferred; the running pipeline polls the
//! session flag each window (attach/detach VAD mid-recording); the
//! `api_apply_live_transcription_now` command flips the flag and spawns the
//! worker on first enable.

use once_cell::sync::Lazy;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, Runtime};
use tokio::sync::mpsc;

use super::pipeline::TranscriptionChunk;
use crate::database::repositories::meeting::MeetingsRepository;
use crate::state::AppState;
use tauri::Manager;

/// Receiver kept alive while a deferred session runs, so a mid-meeting
/// "go live" can hand it to the transcription worker. Dropped at stop.
static PENDING_RECEIVER: Lazy<Mutex<Option<mpsc::Receiver<TranscriptionChunk>>>> =
    Lazy::new(|| Mutex::new(None));

/// The active session's live-STT flag (shared with the pipeline). `None`
/// outside a recording session.
static SESSION_FLAG: Lazy<Mutex<Option<Arc<AtomicBool>>>> = Lazy::new(|| Mutex::new(None));

pub fn stash_receiver(receiver: mpsc::Receiver<TranscriptionChunk>) {
    *PENDING_RECEIVER.lock().unwrap() = Some(receiver);
}

pub fn take_receiver() -> Option<mpsc::Receiver<TranscriptionChunk>> {
    PENDING_RECEIVER.lock().unwrap().take()
}

pub fn set_session_flag(flag: Arc<AtomicBool>) {
    *SESSION_FLAG.lock().unwrap() = Some(flag);
}

pub fn session_flag() -> Option<Arc<AtomicBool>> {
    SESSION_FLAG.lock().unwrap().clone()
}

/// True when the ACTIVE session is currently running live STT.
pub fn session_live_now() -> Option<bool> {
    session_flag().map(|f| f.load(Ordering::SeqCst))
}

/// Stop-time cleanup: drop the flag and any unconsumed receiver.
pub fn clear_session() {
    *SESSION_FLAG.lock().unwrap() = None;
    *PENDING_RECEIVER.lock().unwrap() = None;
}

/// Decide the session's initial live/defer mode (spec §§2–4) and tell the
/// frontend. Called from both recording start paths.
pub async fn decide_session_mode<R: Runtime>(
    app: &AppHandle<R>,
    prefs_live_transcription: bool,
    prefs_low_power_on_battery: bool,
    meeting_id: Option<&str>,
) -> bool {
    let on_battery = crate::power::is_on_battery();
    let override_mode = match meeting_id {
        Some(mid) => {
            let state = app.state::<AppState>();
            let pool = state.db_manager.pool();
            MeetingsRepository::get_processing_mode(pool, mid)
                .await
                .unwrap_or_default()
        }
        None => None,
    };
    let live = super::processing_mode::effective_live_stt(
        prefs_live_transcription,
        prefs_low_power_on_battery,
        on_battery,
        override_mode.as_deref(),
    );
    log::info!(
        "processing mode: live={live} (pref_live={prefs_live_transcription}, low_power={prefs_low_power_on_battery}, on_battery={on_battery}, override={override_mode:?})"
    );
    let _ = app.emit(
        "processing-mode-changed",
        serde_json::json!({
            "meetingId": meeting_id,
            "liveTranscription": live,
            "onBattery": on_battery,
        }),
    );
    live
}
```

(If `TranscriptionChunk` lives elsewhere, follow the type of the receiver returned by `RecordingManager::start_recording` — check `src/audio/recording_manager.rs` — and fix the import.)

- [ ] **Step 2: Ratchet offset — extract device resolution out of `recording_commands.rs`**

Create `src/audio/device_resolution.rs` and move the two duplicated resolution blocks (`start_recording_with_meeting_name` lines ~397–495, and the mirrored block in the second start function near line ~683's flow) into:

```rust
//! Start-time device resolution: preference → default → error/None.
//! Extracted verbatim from the two recording start paths (ratchet offset for
//! the low-power-mode changes; behavior unchanged).

use super::default_devices::{default_input_device, default_output_device}; // fix to the actual module the start paths import these from
use super::devices::{parse_audio_device, AudioDevice}; // likewise
use log::{error, info, warn};
use std::sync::Arc;

/// Microphone: preference → default → Err (required).
pub fn resolve_microphone(preferred: Option<String>) -> Result<Arc<AudioDevice>, String> {
    /* body: the existing lines 400–445, unchanged, returning Arc<AudioDevice>
       instead of Some(Arc<...>) */
    unimplemented!() // replace with the moved code
}

/// System audio: preference → default → None (optional).
pub fn resolve_system_audio(preferred: Option<String>) -> Option<Arc<AudioDevice>> {
    /* body: the existing lines 450–495, unchanged */
    unimplemented!() // replace with the moved code
}
```

Move the code bodies verbatim (adjusting only the return plumbing), declare `pub mod device_resolution;` in `src/audio/mod.rs`, and replace both inline blocks in `recording_commands.rs` with:

```rust
    let microphone_device = Some(device_resolution::resolve_microphone(preferred_mic_name)?);
    let system_device = device_resolution::resolve_system_audio(preferred_system_name);
```

This removes ~190 lines from `recording_commands.rs`, far more than the additions below.

- [ ] **Step 3: Use the effective mode in BOTH start paths**

In `start_recording_with_meeting_name` (and the parallel start function whose prefs-load is at line ~683): after loading preferences, destructure `prefs.low_power_on_battery` too, then replace the plain `live_transcription_enabled` binding with:

```rust
    let live_transcription_enabled = super::live_toggle::decide_session_mode(
        &app,
        live_transcription_enabled,
        low_power_on_battery,
        meeting_id.as_deref(),
    )
    .await;
```

(The second start path may not have a `meeting_id` — pass `None` there.) Everything downstream (model validation gate, live diarizer gate, worker spawn gate) already keys off this variable.

Then change the record-only else-branch (line ~570–573 and its mirror) from `drop(transcription_receiver)` to:

```rust
        info!("🎙️ Deferred mode: transcription worker not spawned (stashing receiver for mid-meeting go-live)");
        super::live_toggle::stash_receiver(transcription_receiver);
```

In the stop path (`stop_recording` in the same file — find where `TRANSCRIPTION_TASK` / listeners are cleaned up), add one line: `super::live_toggle::clear_session();`.

- [ ] **Step 4: Verify**

Run: `cd frontend/src-tauri && cargo check && cargo clippy && cargo test --features metal --lib && wc -l src/audio/recording_commands.rs && cd ../.. && scripts/check-file-size.sh && scripts/check-file-size.sh --update`
Expected: clean; `recording_commands.rs` well under 1558 (the ratchet `--update` tightens its allowlist entry — commit that change too).

- [ ] **Step 5: Commit**

```bash
git add -A frontend/src-tauri/src scripts/file-size-allowlist.txt
git commit -m "feat: recording start picks live/defer via power state and per-meeting override"
```

### Task 6: Mid-meeting go-live / go-defer

**Files:**
- Create: `src/audio/stt_stage.rs`
- Modify: `src/audio/pipeline.rs` (shrink-only 2007 cap — the stt_stage extraction is the offset), `src/audio/recording_manager.rs` (thread the flag; file is 784/800 — keep additions ≤ ~10 lines), `src/audio/live_toggle.rs` (add the command), `src/audio/mod.rs`, `src/registry.rs`
- Test: extend `frontend/src-tauri/tests/pipeline_integration.rs` (exempt from ratchet)

**Interfaces:**
- Consumes: `live_toggle` statics (Task 5), `transcription::{validate_transcription_model_ready, start_transcription_task}` (`src/audio/transcription/`), `ContinuousVadProcessor` (`src/audio/vad.rs`).
- Produces:
  - `SttStage` struct owning the pipeline's VAD/STT gating: `new(sample_rate: u32, live_stt: Arc<AtomicBool>) -> Result<SttStage>`, `is_active(&self) -> bool`, `sync(&mut self)` (attach/detach VAD per the flag), `process(&mut self, mixed: &[f32]) -> Option<anyhow::Result<Vec<SpeechSegment>>>`, `flush(&mut self) -> Option<...>` (same return the pipeline's line-1530 flush consumer expects — read `vad.rs` for the exact types).
  - Tauri command `api_apply_live_transcription_now(enable: bool) -> Result<(), String>` in `live_toggle.rs`.
- Consumed by: Task 10's UI toggle.

- [ ] **Step 1: Read before writing.** Read `src/audio/vad.rs` (the `ContinuousVadProcessor` API: `new(sample_rate, redemption_ms)`, `process_audio`, `flush` and their exact return types), `src/audio/recording_manager.rs` lines 80–220 (how `live_transcription_enabled: bool` flows into `AudioPipeline::new`), and `frontend/src-tauri/tests/pipeline_integration.rs` (the hardware-less harness: how a pipeline is constructed and fed).

- [ ] **Step 2: Implement `SttStage`** (`src/audio/stt_stage.rs`):

```rust
//! The pipeline's VAD/STT gating stage (low-power-mode spec §3).
//!
//! Owns the optional `ContinuousVadProcessor` and a shared `AtomicBool` the
//! session's toggle command flips, so live transcription can attach/detach
//! MID-recording: `sync()` is called once per mix window and lazily
//! constructs (or flushes-and-drops) the VAD to match the flag. Extracted
//! from `pipeline.rs` (specs/0029 WS7.2 record-only gating) — behavior at a
//! fixed flag value is unchanged.

use super::vad::ContinuousVadProcessor;
use anyhow::Result;
use log::{error, info};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// VAD redemption time (ms) — the value previously inlined in
/// `AudioPipeline::new` (same on all platforms).
const REDEMPTION_TIME_MS: u32 = 400;

pub struct SttStage {
    vad: Option<ContinuousVadProcessor>,
    live_stt: Arc<AtomicBool>,
    sample_rate: u32,
}

impl SttStage {
    /// Constructs the VAD up-front when the flag starts true (a start-time
    /// failure must abort the recording start, as before).
    pub fn new(sample_rate: u32, live_stt: Arc<AtomicBool>) -> Result<Self> {
        let vad = if live_stt.load(Ordering::SeqCst) {
            match ContinuousVadProcessor::new(sample_rate, REDEMPTION_TIME_MS) {
                Ok(p) => {
                    info!("VAD-driven pipeline: VAD segments will be sent directly to Whisper (no time-based accumulation)");
                    Some(p)
                }
                Err(e) => {
                    error!("Failed to create VAD processor: {}", e);
                    return Err(anyhow::anyhow!("VAD processor creation failed: {}", e));
                }
            }
        } else {
            info!("🎙️ Deferred mode: VAD/STT stage skipped (audio still recorded; transcribe later)");
            None
        };
        Ok(Self { vad, live_stt, sample_rate })
    }

    pub fn is_active(&self) -> bool {
        self.vad.is_some()
    }

    /// Reconcile the VAD with the session flag. Called once per mix window.
    /// A mid-recording attach failure logs and resets the flag (the session
    /// stays deferred) rather than killing the pipeline.
    pub fn sync(&mut self) {
        let want = self.live_stt.load(Ordering::SeqCst);
        if want && self.vad.is_none() {
            match ContinuousVadProcessor::new(self.sample_rate, REDEMPTION_TIME_MS) {
                Ok(p) => {
                    info!("🎙️ Live transcription enabled mid-recording — VAD/STT stage attached");
                    self.vad = Some(p);
                }
                Err(e) => {
                    error!("Mid-recording VAD attach failed (staying deferred): {e}");
                    self.live_stt.store(false, Ordering::SeqCst);
                }
            }
        } else if !want && self.vad.is_some() {
            // Detach. The un-flushed tail (< ~1 s) is dropped deliberately —
            // a deferred meeting gets a full retranscription later anyway.
            info!("🎙️ Live transcription disabled mid-recording — VAD/STT stage detached");
            self.vad = None;
        }
    }

    pub fn process(&mut self, mixed: &[f32]) -> Option<Result<Vec<super::vad::SpeechSegment>>> {
        self.vad.as_mut().map(|vad| vad.process_audio(mixed))
    }

    pub fn flush(&mut self) /* -> match the existing line-1530 consumer's type */ {
        // Delegate to vad.flush() exactly as pipeline.rs:1530 did; copy the
        // signature from there after reading vad.rs.
    }
}
```

Fix the `SpeechSegment`/flush types to whatever `vad.rs` actually exports. Declare `pub mod stt_stage;` in `src/audio/mod.rs`.

- [ ] **Step 3: Swap it into `AudioPipeline`** (`src/audio/pipeline.rs` — net-negative edit):

- `new(...)`: parameter `live_transcription_enabled: bool` becomes `live_stt: Arc<AtomicBool>`; the whole `vad_processor` construction block (lines ~1026–1052) collapses to `let stt_stage = super::stt_stage::SttStage::new(sample_rate, live_stt)?;`; field `vad_processor` is replaced by `stt_stage: SttStage`.
- Line ~1381: `if self.vad_processor.is_some()` → `if self.stt_stage.is_active()`.
- Lines ~1392–1397: insert `self.stt_stage.sync();` immediately before, then `let vad_result = self.stt_stage.process(&mixed_with_gain);`. **`sync()` must run before the `is_active()` channel-window check** so the clock starts advancing the same window the stage attaches — reorder to: `self.stt_stage.sync(); if self.stt_stage.is_active() { self.record_channel_window(...) } let vad_result = self.stt_stage.process(...)`.
- Line ~1530 flush: delegate through `self.stt_stage.flush()`.
- The second `AudioPipeline` construction site (~line 1640–1675) threads the `Arc<AtomicBool>` instead of the bool.

- [ ] **Step 4: Thread the flag through the manager** (`src/audio/recording_manager.rs`): `start_recording(..., live_transcription_enabled: bool)` (lines 86, 145) becomes `live_stt: Arc<AtomicBool>`; where it previously logged/branched on the bool, read `live_stt.load(Ordering::SeqCst)` into a local at the top. In `recording_commands.rs`, both start paths build it and register it:

```rust
    let live_stt = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(live_transcription_enabled));
    super::live_toggle::set_session_flag(live_stt.clone());
```
and pass `live_stt` to `manager.start_recording(...)`. (These few added lines are covered by Task 5's extraction headroom.)

- [ ] **Step 5: The toggle command** (append to `src/audio/live_toggle.rs`):

```rust
/// Flip live transcription for the ACTIVE recording session (spec §3).
/// Enabling validates the model, spawns the transcription worker on first
/// use (handing it the stashed receiver), then raises the flag; the pipeline
/// attaches its VAD on the next mix window. Disabling just lowers the flag
/// (the worker idles; the engine stays resident — CPU, not RAM, is the
/// battery cost). Persisting the meeting's `processing_mode` override is the
/// frontend's separate `api_set_meeting_processing_mode` call.
#[tauri::command]
pub async fn api_apply_live_transcription_now<R: Runtime>(
    app: AppHandle<R>,
    enable: bool,
) -> Result<(), String> {
    let Some(flag) = session_flag() else {
        return Err("No recording in progress".to_string());
    };
    if enable {
        super::transcription::validate_transcription_model_ready(&app).await?;
        if let Some(receiver) = take_receiver() {
            let handle = super::transcription::start_transcription_task(app.clone(), receiver);
            super::recording_commands::store_transcription_task(handle);
        }
        flag.store(true, std::sync::atomic::Ordering::SeqCst);
    } else {
        flag.store(false, std::sync::atomic::Ordering::SeqCst);
    }
    let _ = app.emit(
        "processing-mode-changed",
        serde_json::json!({
            "liveTranscription": enable,
            "onBattery": crate::power::is_on_battery(),
        }),
    );
    Ok(())
}
```

`store_transcription_task` is a 4-line pub helper to add in `recording_commands.rs` next to the `TRANSCRIPTION_TASK` static (locks it and stores the handle) — needed because the static is private to that file. Register the command in `registry.rs` (audio section).

- [ ] **Step 6: Integration test** — add to `frontend/src-tauri/tests/pipeline_integration.rs`, following its existing harness, a test `mid_stream_live_toggle_attaches_and_detaches_stt`:
  1. Build a pipeline with `live_stt = Arc::new(AtomicBool::new(false))` and a captured transcription channel; feed ~2 s of the harness's speech fixture; assert **zero** `TranscriptionChunk`s arrived.
  2. `flag.store(true, ..)`; feed the same fixture again; assert ≥ 1 chunk arrives.
  3. `flag.store(false, ..)`; feed again; assert no NEW chunks after a drain.
  Use the harness's existing fixture-generation (`say`-based) and channel-wiring helpers; do not invent new audio plumbing.

- [ ] **Step 7: Verify**

Run: `cd frontend/src-tauri && cargo clippy && cargo test --features metal --test pipeline_integration && cargo test --features metal --lib && wc -l src/audio/pipeline.rs && cd ../.. && scripts/check-file-size.sh --update`
Expected: PASS; `pipeline.rs` ≤ its (possibly tightened) allowlist entry.

- [ ] **Step 8: Commit**

```bash
git add -A frontend/src-tauri scripts/file-size-allowlist.txt
git commit -m "feat: mid-recording live-transcription toggle (SttStage attach/detach)"
```

### Task 7: Deferred bookkeeping — backlog query + retention exemption

**Files:**
- Modify: `src/audio/retention.rs` (479 lines, headroom), `src/database/repositories/meeting/query.rs`
- Create: command in `src/audio/deferred_backlog.rs` + `src/audio/mod.rs` declaration + `src/registry.rs` registration

**Interfaces:**
- Consumes: `audio::constants::AUDIO_EXTENSIONS`, `MeetingModel.processing_mode`, retention's `SweepCandidate`/`decide_sweep`.
- Produces:
  - `MeetingsRepository::list_deferred_candidates(pool) -> Result<Vec<DeferredCandidateRow>, SqlxError>` where `DeferredCandidateRow { id: String, title: String, folder_path: Option<String>, transcript_count: i64, processing_mode: Option<String>, created_at: String }` (FromRow).
  - Tauri command `api_list_deferred_meetings() -> Result<Vec<DeferredMeeting>, String>` with `DeferredMeeting { id, title, folderPath, transcriptCount }` (camelCase serialize) — only rows whose folder actually contains an audio file.
  - `SweepCandidate.processing_mode: Option<String>` + `decide_sweep` exemption.
- Consumed by: Task 9 (frontend backlog).

- [ ] **Step 1: Failing tests for the sweep exemption** — in `retention.rs`'s existing `#[cfg(test)]` mod (read it first; follow its style):

```rust
    #[test]
    fn deferred_meeting_is_exempt_even_with_many_transcripts() {
        // Live→Defer mid-meeting leaves >MIN_TRANSCRIPT_SEGMENTS rows but the
        // meeting still awaits its full deferred pass — must never be swept.
        let candidate = SweepCandidate {
            meeting_id: "m1".into(),
            created_at: Utc::now() - Duration::days(100),
            transcript_count: 50,
            processing_mode: Some("defer".into()),
        };
        assert_eq!(
            decide_sweep(Utc::now(), Some(30), &candidate),
            SweepDecision::ExemptUntranscribed
        );
    }
```

(Existing tests gain `processing_mode: None` in their candidate literals.)

- [ ] **Step 2: Implement** — add the field to `SweepCandidate`; in `decide_sweep`, after the `transcript_count` check:

```rust
    if candidate.processing_mode.as_deref() == Some(crate::audio::processing_mode::MODE_DEFER) {
        return SweepDecision::ExemptUntranscribed;
    }
```

and populate the field in the sweeper's candidate-building query (find it below `decide_sweep` in the same file — add `processing_mode` to its SELECT).

- [ ] **Step 3: Backlog query** — in `src/database/repositories/meeting/query.rs`:

```rust
/// One candidate for deferred backlog processing (low-power-mode spec §5).
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct DeferredCandidateRow {
    pub id: String,
    pub title: String,
    pub folder_path: Option<String>,
    pub transcript_count: i64,
    pub processing_mode: Option<String>,
    pub created_at: String,
}

impl MeetingsRepository {
    /// Meetings with recorded audio whose processing is pending: explicitly
    /// deferred (`processing_mode='defer'`) or effectively untranscribed
    /// (sparse transcript — same threshold as the retention exemption).
    /// The caller filters by on-disk audio presence.
    pub async fn list_deferred_candidates(
        pool: &SqlitePool,
    ) -> Result<Vec<DeferredCandidateRow>, sqlx::Error> {
        sqlx::query_as::<_, DeferredCandidateRow>(
            "SELECT m.id, m.title, m.folder_path, m.processing_mode, m.created_at,
                    (SELECT COUNT(*) FROM transcripts t WHERE t.meeting_id = m.id) AS transcript_count
             FROM meetings m
             WHERE m.folder_path IS NOT NULL
               AND (m.processing_mode = 'defer'
                    OR (SELECT COUNT(*) FROM transcripts t WHERE t.meeting_id = m.id) < ?)
             ORDER BY m.created_at ASC",
        )
        .bind(crate::audio::retention::MIN_TRANSCRIPT_SEGMENTS)
        .fetch_all(pool)
        .await
    }
}
```

(Place inside the existing `impl MeetingsRepository` block in that file.)

- [ ] **Step 4: The command** — `src/audio/deferred_backlog.rs`:

```rust
//! Deferred-backlog query (low-power-mode spec §5). The debounce/prompt/
//! processing loop lives in the frontend; this command answers "which
//! meetings still need processing and still have audio on disk?".

use crate::audio::constants::AUDIO_EXTENSIONS;
use crate::database::repositories::meeting::MeetingsRepository;
use crate::state::AppState;
use serde::Serialize;
use std::path::Path;
use tauri::{AppHandle, Manager, Runtime};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeferredMeeting {
    pub id: String,
    pub title: String,
    pub folder_path: String,
    pub transcript_count: i64,
}

/// True when the meeting folder still contains at least one media file.
fn folder_has_audio(folder: &Path) -> bool {
    std::fs::read_dir(folder)
        .map(|entries| {
            entries.flatten().any(|e| {
                let p = e.path();
                p.is_file()
                    && p.extension()
                        .map(|ext| {
                            AUDIO_EXTENSIONS.contains(&ext.to_string_lossy().to_lowercase().as_str())
                        })
                        .unwrap_or(false)
            })
        })
        .unwrap_or(false)
}

#[tauri::command]
pub async fn api_list_deferred_meetings<R: Runtime>(
    app: AppHandle<R>,
) -> Result<Vec<DeferredMeeting>, String> {
    let pool = {
        let state = app.state::<AppState>();
        state.db_manager.pool().clone()
    };
    let rows = MeetingsRepository::list_deferred_candidates(&pool)
        .await
        .map_err(|e| format!("Failed to list deferred meetings: {e}"))?;
    Ok(rows
        .into_iter()
        .filter_map(|r| {
            let folder = r.folder_path?;
            folder_has_audio(Path::new(&folder)).then(|| DeferredMeeting {
                id: r.id,
                title: r.title,
                folder_path: folder,
                transcript_count: r.transcript_count,
            })
        })
        .collect())
}
```

Declare in `src/audio/mod.rs`, register in `registry.rs`. If `retention::MIN_TRANSCRIPT_SEGMENTS` isn't already `pub`, it is (line 30) — reuse it, don't duplicate the constant.

- [ ] **Step 5: Verify + commit**

Run: `cd frontend/src-tauri && cargo clippy && cargo test --features metal --lib && cd ../.. && scripts/check-file-size.sh`
Expected: PASS.

```bash
git add -A frontend/src-tauri/src
git commit -m "feat: deferred-backlog query and retention exemption for deferred meetings"
```

### Task 8: Frontend session state — mode events, stop-time bookkeeping

**Files:**
- Create: `src/lib/processing-mode.ts` (pure helpers), `src/hooks/useProcessingMode.ts`
- Modify: `src/hooks/useRecordingStop.ts` (~600 lines — check `wc -l` stays ≤ 800)
- Test: `src/lib/__tests__/processing-mode.test.ts`

**Interfaces:**
- Consumes: events `processing-mode-changed` (Tasks 5–6), commands `api_get_meeting_processing_mode`, `api_set_meeting_processing_mode` (Task 4).
- Produces:
  - `processing-mode.ts`: `const SESSION_DEFERRED_KEY = 'recording_session_started_deferred'`; `markSessionDeferred(deferred: boolean)` / `sessionStartedDeferred(): boolean` (sessionStorage-backed, resilient to unavailable storage); `stopAction(startedDeferred: boolean, overrideMode: string | null): 'process-now' | 'mark-defer' | 'none'` — pure: `'process-now'` when `startedDeferred && overrideMode === 'live'`; `'mark-defer'` when a session ends still-deferred (`startedDeferred && overrideMode !== 'live'`) OR `overrideMode === 'defer'`; else `'none'`.
  - `useProcessingMode()` hook: subscribes to `processing-mode-changed`, exposes `{ liveTranscription: boolean | null, onBattery: boolean | null }`, and calls `markSessionDeferred(!payload.liveTranscription)` when a payload carries a `meetingId` (i.e. the start-time emit).
- Consumed by: Tasks 9–10.

- [ ] **Step 1: Failing Vitest tests** (`src/lib/__tests__/processing-mode.test.ts`):

```ts
import { describe, expect, it } from 'vitest';
import { stopAction } from '../processing-mode';

describe('stopAction', () => {
  it('processes immediately when a deferred session was overridden live', () => {
    expect(stopAction(true, 'live')).toBe('process-now');
  });
  it('marks a still-deferred session for backlog', () => {
    expect(stopAction(true, null)).toBe('mark-defer');
  });
  it('marks a live session the user deferred mid-meeting', () => {
    expect(stopAction(false, 'defer')).toBe('mark-defer');
  });
  it('does nothing for a normal live meeting', () => {
    expect(stopAction(false, null)).toBe('none');
  });
});
```

Run: `cd frontend && pnpm test -- processing-mode` → FAIL (module missing).

- [ ] **Step 2: Implement `src/lib/processing-mode.ts` + `src/hooks/useProcessingMode.ts`** per the interface above (mirror the listen/unlisten cleanup pattern of an existing small hook such as `useDiarization.ts`). Note the esbuild/Vitest gotcha from the project memory: if `pnpm test` prompts about esbuild build scripts, it has been approved before — just re-run.

- [ ] **Step 3: Stop-time bookkeeping** — in `useRecordingStop.ts`, right after the meeting save succeeds (immediately before the auto-diarization block at line ~470), add:

```ts
          // Low-power-mode bookkeeping (spec §§3,5): decide this meeting's fate.
          try {
            const overrideMode = await invoke<string | null>('api_get_meeting_processing_mode', { meetingId });
            const action = stopAction(sessionStartedDeferred(), overrideMode);
            if (action === 'mark-defer') {
              // Backlog picks it up (and retention must not sweep it).
              await invoke('api_set_meeting_processing_mode', { meetingId, mode: 'defer' });
            } else if (action === 'process-now') {
              // Overridden live mid-meeting: full uniform pass now, battery or not.
              window.dispatchEvent(
                new CustomEvent('process-deferred-meeting-now', { detail: { meetingId } })
              );
            }
          } catch (error) {
            console.warn('Processing-mode bookkeeping failed (meeting saved fine):', error);
          } finally {
            markSessionDeferred(false);
          }
```

The `process-deferred-meeting-now` window event is consumed by Task 9's backlog manager (mirrors the existing `stop-recording-from-global-bar` window-event pattern). For the `'process-now'` case ALSO skip the existing auto-diarization block (the full pass includes diarization): guard it with `if (action !== 'process-now')` by hoisting `action` above it.

- [ ] **Step 4: Verify + commit**

Run: `cd frontend && pnpm lint && pnpm test`
Expected: PASS (new tests green).

```bash
git add frontend/src/lib/processing-mode.ts frontend/src/lib/__tests__/processing-mode.test.ts \
        frontend/src/hooks/useProcessingMode.ts frontend/src/hooks/useRecordingStop.ts
git commit -m "feat: session processing-mode tracking and stop-time defer/process-now bookkeeping"
```

### Task 9: Backlog prompt + sequential processor (frontend)

**Files:**
- Create: `src/lib/deferred-backlog.ts` (pure queue/prompt logic), `src/hooks/useDeferredBacklog.ts`, `src/components/DeferredBacklog/DeferredBacklogPrompt.tsx`
- Modify: `src/app/layout.tsx` (mount the prompt component next to `<GlobalRecordingBar />`)
- Test: `src/lib/__tests__/deferred-backlog.test.ts`

**Interfaces:**
- Consumes: `power-source-changed` event + `api_get_power_state` (Task 3), `api_list_deferred_meetings` (Task 7), `start_retranscription_command` / `retranscription-complete` / `retranscription-error`, `api_get_diarization_enabled` + `api_diarization_models_present` + `api_diarize_meeting` + `diarization-complete`/`diarization-error` events (see `useDiarization.ts` for names), `api_process_transcript` + summary polling (read `useSummaryGeneration.ts:178–310` and its `startSummaryPolling` import — reuse the same context/module), `retranscriptionProviderFor` from `src/lib/deferred-transcription.ts`, `api_set_meeting_processing_mode` (clear on success), `window` event `process-deferred-meeting-now` (Task 8).
- Produces: `shouldPromptBacklog(onBattery: boolean, isRecording: boolean, count: number): boolean` (pure); `useDeferredBacklog()` managing `{ pending, processing, currentIndex, progressLabel, accept, decline, dismissBadge }`; the prompt UI.

- [ ] **Step 1: Failing Vitest tests** for the pure logic (`deferred-backlog.test.ts`): `shouldPromptBacklog` is true only when `!onBattery && !isRecording && count > 0`; plus a small reducer test if you model the queue as a reducer.

- [ ] **Step 2: Implement the hook.** Behavior spec:
  - On mount (app layout, post-onboarding): `api_get_power_state`; if on AC, schedule a check after **120 s** (startup grace, mirrors the retention sweeper's rationale).
  - On `power-source-changed` to AC: schedule a check after **90 s**; cancel the timer if a `power-source-changed` back to battery arrives, or a recording starts (`useRecordingState().isRecording`).
  - A "check" = `api_list_deferred_meetings`; if `shouldPromptBacklog(...)` → show the prompt (toast-like banner, count + "Process now" / "Later"). "Later" collapses to a small badge on the banner area; re-prompt only on the next AC transition.
  - **Accept** → process meetings SEQUENTIALLY. Per meeting:
    1. If `transcriptCount < SPARSE_TRANSCRIPT_SEGMENTS` OR the meeting was explicitly deferred: `start_retranscription_command` (`meetingId`, `meetingFolderPath`, provider via `retranscriptionProviderFor(...)` from ConfigContext's transcript config, language/model null → backend defaults); await `retranscription-complete` (resolve) / `retranscription-error` (skip meeting, continue) — copy the listen-once pattern from `useSummaryGeneration.ts:515–560`.
    2. Diarize best-effort, exactly mirroring `useRecordingStop.ts:470–480` gating (`api_get_diarization_enabled` && `api_diarization_models_present`), then await the completion event per `useDiarization.ts`.
    3. Summarize: mirror `processSummary`'s `api_process_transcript` invocation (fetch transcripts the way `fetchAllTranscripts` does at `useSummaryGeneration.ts:455`, join text the way `buildSummaryTranscriptPayload` does at `:587`, resolve the meeting's template via `api_get_meeting_template`, resolve language via the same `resolveSummaryLanguage` helper) and await completion via the same `startSummaryPolling` mechanism `useSummaryGeneration` uses. If no summary provider is configured, skip with a note in the final toast ("summaries skipped — no provider configured").
    4. On success: `api_set_meeting_processing_mode(meetingId, null)` to clear the pending marker.
  - Also listen for the `process-deferred-meeting-now` window event (Task 8) and run the same per-meeting pipeline immediately for that single meeting (no prompt, no power check — the user explicitly overrode).
  - Abort processing if a recording starts; whatever completed stays completed.
  - Never grow `useSummaryGeneration.ts` (866 cap): if reuse requires exporting a helper from it, EXTRACT that helper into `src/lib/` and re-import it from both places (net shrink of the hook), don't add to it.

- [ ] **Step 3: The prompt component** (`DeferredBacklogPrompt.tsx`): a fixed-position banner styled consistently with `GlobalRecordingBar.tsx` (reuse its container classes); shows count, per-meeting progress ("Processing 2 of 3 — transcribing 'Weekly sync'…"), and Process-now/Later buttons. Mount in `layout.tsx` adjacent to `<GlobalRecordingBar />` so it exists on every route.

- [ ] **Step 4: Verify**

Run: `cd frontend && pnpm lint && pnpm test`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add frontend/src/lib/deferred-backlog.ts frontend/src/lib/__tests__/deferred-backlog.test.ts \
        frontend/src/hooks/useDeferredBacklog.ts frontend/src/components/DeferredBacklog/ frontend/src/app/layout.tsx
git commit -m "feat: AC-return backlog prompt with sequential transcribe/diarize/summarize"
```

### Task 10: Settings toggle, recording indicator, per-meeting control

**Files:**
- Modify: `src/components/RecordingSettings.tsx` (707 + ~45 lines — stays ≤ 800), `src/app/_components/TranscriptPanel.tsx`, `src/components/Record/RecordingHeader.tsx`
- Test: extend `src/lib/__tests__/processing-mode.test.ts` with any new pure copy/format helpers

**Interfaces:**
- Consumes: `useProcessingMode` (Task 8), `api_set_meeting_processing_mode` + `api_apply_live_transcription_now` (Tasks 4, 6), `useSidebar().activeRecordingMeetingId`, `RecordingPreferences.low_power_on_battery` (Task 4).
- Produces: user-facing controls; no new programmatic interfaces.

- [ ] **Step 1: Settings toggle** — in `RecordingSettings.tsx`: add `low_power_on_battery: boolean` to its local `RecordingPreferences` interface (line ~28) and default state (line ~48, `true`); add `handleLowPowerToggle` mirroring `handleLiveTranscriptionToggle` (line ~308) with toast copy:
  - on: `'On battery power, meetings are recorded only — transcription and summaries wait until you're plugged in.'`
  - off: `'Meetings are transcribed live even on battery.'`

  and a switch row directly below the live-transcription switch (line ~497), labeled **"Low Power Mode on battery"** with description *"When on battery, record audio but defer transcription and summaries until you're back on power. You can override per meeting while recording."*

- [ ] **Step 2: Recording empty-state** — `TranscriptPanel.tsx` currently keys its record-only empty state off the stored preference (lines 39–45, 151). Replace that with `useProcessingMode()`'s live value (fall back to the pref read until the first event arrives). When the session is deferred AND `onBattery`, the empty-state copy becomes: *"Low Power Mode — recording only. Transcription resumes when you're plugged in, or turn it on for this meeting."* with a button **"Transcribe this meeting live"** that calls:

```ts
    await invoke('api_set_meeting_processing_mode', { meetingId: activeRecordingMeetingId, mode: 'live' });
    await invoke('api_apply_live_transcription_now', { enable: true });
```

(guard on `activeRecordingMeetingId`; on error, toast and revert nothing — state comes back via the `processing-mode-changed` event).

- [ ] **Step 3: Header toggle** — in `Record/RecordingHeader.tsx`, next to the `ParticipantsPopover` (line ~228), add a compact mode chip driven by `useProcessingMode()`: shows **"Deferred"** (with a battery glyph when `onBattery`) or **"Live"**; clicking flips it — to live: the two invokes above; to defer: `api_set_meeting_processing_mode(.., 'defer')` + `api_apply_live_transcription_now(false)`. Disable the chip (tooltip "waiting for model…") while the enable invoke is in flight.

- [ ] **Step 4: Verify + commit**

Run: `cd frontend && pnpm lint && pnpm test`
Expected: PASS.

```bash
git add frontend/src/components/RecordingSettings.tsx frontend/src/app/_components/TranscriptPanel.tsx \
        frontend/src/components/Record/RecordingHeader.tsx frontend/src/lib/__tests__/processing-mode.test.ts
git commit -m "feat: low-power settings toggle, deferred indicator, per-meeting live/defer control"
```

### Task 11: Full gate + smoke test

**Files:** none new (fixes only as needed)

- [ ] **Step 1: Full Definition-of-Done gate**

```bash
cd frontend/src-tauri && source ~/.cargo/env && cargo check && cargo clippy && cargo test --features metal
cd ../ && pnpm lint && pnpm test
cd ../ && scripts/check-file-size.sh
```
Expected: all clean except the known `vad_filter` macOS 15.x failure (pre-existing — verify it is the ONLY test failure).

- [ ] **Step 2: Manual smoke test** (launch via `cd frontend && ./dev-vinyl.sh`, or use the `run-vinyl` skill):
  1. On AC: record a short meeting → live transcript streams (unchanged classic path).
  2. Toggle "Low Power Mode on battery" off/on in Settings → toasts correct.
  3. Simulate battery: unplug (or on a desktop, temporarily hard-code `is_on_battery()` to `true` — REVERT before commit). Record → no live transcript; empty state shows the low-power copy; header chip shows "Deferred".
  4. Mid-meeting, click "Transcribe this meeting live" → live transcript starts within a few seconds; stop → full retranscription then diarization then summary run.
  5. Record another deferred meeting, leave it deferred, stop. Replug AC → within ~90 s the backlog prompt appears; Process now → transcript, speakers, summary appear; the meeting no longer lists in a fresh `api_list_deferred_meetings`.
  6. During a recording, open attendees, remove one, stop recording, open the meeting details → the removed attendee is still gone; re-add them → they return.

- [ ] **Step 3: Final commit**

```bash
git add -A && git commit -m "chore: low-power-mode gate fixes"   # only if the gate required fixes
```

---

## Self-review notes (already applied)

- Spec §3's "merge by timestamp" was superseded in the spec itself by full-retranscribe-replace (retranscription already deletes-and-reinserts); the plan implements the updated spec.
- Spec §5's backend watcher was simplified to frontend debounce + two backend query commands (spec updated).
- The mid-meeting detach keeps the STT engine resident (idle) rather than unloading — CPU, not RAM, is the battery cost (spec updated).
- Live diarization is NOT attached on a mid-meeting go-live (it would have no pre-toggle segments to align); the post-meeting diarization pass covers it. The `live_diarizer` start-gate keeps its existing behavior.
- Mid-meeting go-live restarts the VAD clock at the toggle point, so live segment `audio_start_time`s are toggle-relative; acceptable because the final transcript always comes from the full retranscription pass.
