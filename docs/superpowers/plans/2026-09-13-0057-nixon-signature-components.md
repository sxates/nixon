# Nixon Signature Components (transport rail, VU, counter, reels, channel strip, global queue) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the three inconsistent recording-control surfaces and the two competing status pills with one persistent bottom **transport rail** (REC/HOLD/STOP keys, reels, tape counter, level ladder, and ONE global queue), put a needle **VU meter** on the live Record screen, and turn the speaker chip cloud into a **channel strip** with share-of-talk. First, fix the recordings write-root so it reads the persisted preference.

**Architecture:** New components live under `frontend/src/components/Transport/` (rail, keys, counter, reels, ladder, VU, lamps, queue) and `MeetingDetails/ChannelStrip.tsx`, each a small file with a pure helper module and a test. The rail is mounted once in `app/layout.tsx` (replacing `GlobalRecordingBar` and `DeferredBacklogIndicator`), reads `RecordingStateContext` for status/elapsed, `useBacklog()` + `useLlmActivity()` for the queue, and drives the existing start/pause/stop paths (`useSidebar().handleRecordingToggle`, `recordingService.pause/resumeRecording`, `requestFullRecordingStop`). The VU meter is a second-order integrator on `requestAnimationFrame` fed by the existing `recording-level` event. Pages reserve rail height through one CSS variable. On the Rust side, an "active recordings root" cache set from the persisted preference replaces the six direct calls to the filesystem probe.

**Tech Stack:** Tauri 2 (Rust, tauri-plugin-store), Next.js 14 / React 18 / TypeScript, Tailwind 3 with the Nixon semantic tokens (`brand`, `record`, `success`, `destructive`, `chart-1..8`, `panel`, `engrave`, `key`, `well`, `walnut`, `meter-over`), Radix Popover, Vitest + Testing Library (jsdom).

**Spec:** `specs/0057-nixon-rebrand-and-tape-deck-redesign.md` — "Decisions locked" (binding; especially 6, 7, 8, 9), §3 (signature components), §4 Record/Meeting details, and "Plan 1 residuals". This plan is spec §7 **Phase C**. Plan 1 (`docs/superpowers/plans/2026-09-13-0057-nixon-foundation.md`) is already on branch `feat/0057-nixon-rebrand`; this plan continues on the same branch.

## Global Constraints

- Branch `feat/0057-nixon-rebrand`; no merge, no push (spec decision 12).
- **One persistent bottom rail on every screen**, 44px tall, flush to the bottom edge, spanning from the sidebar's right edge to the window edge. It is the ONLY place recording controls live (decision 7). No floating pills, no header buttons.
- **One global queue** (decision 8): deferred transcription/diarization/summary items AND background LLM tasks in one ordered list, in the rail's right zone, with a lamp (amber = running, red = failure latched until dismissed). The sidebar `LlmActivityRow` is deleted. `ProcessMeetingsButton` on Today stays.
- **VU**: needle on the Record screen with 300 ms-to-99% ballistics, ≤1.5% overshoot, integrator stepped on rAF from `recording-level` `{rms, peak}` (12.5 Hz). A **10-segment ladder** everywhere compact (rail, device picker). The spectrometer (`recording-spectrum`, `useRecordingWaveform`) is retired from the UI (decision 6); the Rust emit stays.
- **Counter**: mechanical roller digits, always `h:mm:ss` with leading zeros, tabular Archivo; sizes 28px (record header — NOT used in Plan 2 since the header loses its counter, decision 9), 15px rail, 13px list rows.
- **Reels**: two hubs + tape path; turn at 0.85 rev/s while recording, stop dead on HOLD, gated by `prefers-reduced-motion` (static + amber lamp when reduced).
- **Record header** (decision 9): title + identity line left; VU meter + PEAK / MIC GATE lamps right; no counter, no reels, no status chip, no spectrometer.
- **Channel strip**: fixed-width rows `CH · color bar · name · time · share ladder · %`; CH1 is always the owner mic; rename/merge/assign behavior preserved by reusing `SpeakerChip`.
- State vocabulary (spec §3.1 table): idle / recording / paused(HOLD) / mic-gated / finalizing. REC lamp = `record`, HOLD lamp = `brand`, STOP never lit. Every existing `aria-label` on pause/resume/stop is preserved verbatim: `"Pause recording"`, `"Resume recording"`, `"Stop recording"`; the rail region keeps `aria-label="Recording in progress"` while a session is in flight.
- Semantic tokens only; `scripts/check-off-token-colors.sh` must stay green. File-size ratchet: new files only for new code; `SpeakerLegend.tsx` (689 lines) may not grow; never bare `cargo fmt`.
- Motion: key press 60 ms, lamp attack 120 ms / decay 400 ms, counter digit roll 110 ms, reels 0.85 rev/s constant, route/panel transitions opacity-only 140 ms. No springs, no bounce.
- Definition of Done per task: named commands pass. Plan DoD: `cd frontend/src-tauri && cargo check --features metal && cargo clippy --features metal --all-targets -- -D warnings && cargo test --features metal` (known pre-existing red: `vad_filter` adversarial-fixtures on macOS 15); `cd frontend && pnpm lint && pnpm test && npx tsc --noEmit` (known pre-existing: one `bun:test` error, gate = no new errors); `scripts/check-file-size.sh`; `scripts/check-off-token-colors.sh`; app launches via `frontend/dev-nixon.sh`; owner smoke: record → HOLD → resume → STOP → summary, in both themes.
- Commit trailer on every commit:
  ```
  Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
  Claude-Session: https://claude.ai/code/session_01S81GbiVj2rm2JNeZqSn8vX
  ```

---

## File map

| Area | Files |
|---|---|
| Write root (Rust) | Modify `frontend/src-tauri/src/audio/recording_preferences.rs` (add `recordings_root()` + cache), `lib.rs` (init in setup), call sites `audio/recording_saver.rs:515`, `audio/import.rs:329`, `audio/recording_recovery.rs:289,322`, `audio/processing_reconcile.rs:121`, `diarization/pipeline.rs:212`; scripts `frontend/dev-nixon.sh`, `frontend/upgrade-nixon.sh` (secret check) |
| Rail height | `frontend/src/app/globals.css` (`--rail-h`), `components/MainContent/index.tsx`, every `h-screen` page root (`app/page.tsx:197`, `settings/page.tsx:77`, `tasks/page.tsx:301`, `record/page.tsx:139`, `ask/page.tsx:364`, `saved-question/page.tsx:267`, `meeting-details/page-content.tsx:176`, `person-details/page.tsx:376`, `people/page.tsx:301`, `meetings/page.tsx:324`) |
| Transport primitives | Create `components/Transport/TransportKey.tsx`, `LampDot.tsx`, `TapeCounter.tsx`, `Reels.tsx`, `LevelLadder.tsx`, `VuMeter.tsx`; helpers `lib/transport/format-elapsed.ts`, `lib/transport/vu-ballistics.ts`, `lib/transport/ladder.ts`; tests under `lib/transport/__tests__/` and `components/Transport/__tests__/` |
| Level feed | Create `hooks/useRecordingLevel.ts` (listens `recording-level`), `hooks/useMicGate.ts` (listens `zoom-mute-changed`) |
| Rail | Create `components/Transport/TransportRail.tsx`, `TransportStatus.tsx`, `QueueIndicator.tsx`, `QueuePanel.tsx`; `lib/transport/queue-view.ts` (merges backlog + LLM activity) + test |
| Mount | `app/layout.tsx` (mount rail; delete `GlobalRecordingBar` + `DeferredBacklogIndicator`; LlmActivityProvider wraps Sidebar + rail) |
| Record screen | `app/record/page.tsx` (drop floating controls, spectrometer bars, framer entrance), `components/Record/RecordingHeader.tsx` (drop chip/timer/spectrometer/controls; add VU + lamps), extract `components/Record/RecordingDeviceAlert.tsx` + `lib/recording-start-errors.ts` from `RecordingControls.tsx`; `hooks/useRecordingStart.ts` (use the error mapper) |
| Delete | `components/RecordingControls.tsx`, `components/GlobalRecordingBar.tsx`, `components/DeferredBacklog/DeferredBacklogIndicator.tsx`, `components/LlmActivity/LlmActivityRow.tsx` + its test, `components/AudioLevelMeter.tsx`, `hooks/useRecordingWaveform.ts` + its test |
| Device picker | `components/DeviceSelection.tsx` (AudioLevelMeter → LevelLadder) |
| Channel strip | Create `components/MeetingDetails/ChannelStrip.tsx`, `lib/speaker-talk-time.ts` + test; modify `SpeakerLegend.tsx` (replace the chip-cloud container), `MeetingDetails/TranscriptPanel.tsx:300-305` (pass transcripts) |
| Docs | `CHANGELOG.md` Unreleased; `docs/MANUAL_SMOKE.md` (rail smoke steps) |

---

### Task 1: Recordings write root reads the persisted preference (Rust) + `.env.google` secret check

**Files:**
- Modify: `frontend/src-tauri/src/audio/recording_preferences.rs`, `frontend/src-tauri/src/audio/mod.rs:117`, `frontend/src-tauri/src/lib.rs` (setup hook), `audio/recording_saver.rs:515`, `audio/import.rs:329`, `audio/recording_recovery.rs:289,322`, `audio/processing_reconcile.rs:121-123`, `diarization/pipeline.rs:209-212`, `frontend/dev-nixon.sh`, `frontend/upgrade-nixon.sh`

**Interfaces:**
- Produces: `pub fn recordings_root() -> PathBuf` (the active write root: the cached persisted `save_folder` if set, else `get_default_recordings_folder()`), `pub fn set_recordings_root(root: PathBuf)`, `pub async fn init_recordings_root<R: Runtime>(app: &AppHandle<R>)` (loads prefs and calls `set_recordings_root`). `save_recording_preferences` calls `set_recordings_root(preferences.save_folder.clone())` after a successful store save. All six call sites use `recordings_root()`.

- [ ] **Step 1: Write the failing tests**

Append to the existing `mod tests` in `recording_preferences.rs`:
```rust
    #[test]
    fn recordings_root_falls_back_to_the_probe_when_unset() {
        // Fresh process state: nothing cached yet (this test must not run after a set — see
        // the `serial` note below). Use a private helper that takes the cache explicitly so
        // the test is order-independent.
        let cache = RecordingsRootCache::default();
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(
            cache.resolve_with(|| tmp.path().join("probe")),
            tmp.path().join("probe")
        );
    }

    #[test]
    fn recordings_root_prefers_the_cached_persisted_folder() {
        let cache = RecordingsRootCache::default();
        let tmp = tempfile::tempdir().unwrap();
        cache.set(tmp.path().join("chosen"));
        assert_eq!(
            cache.resolve_with(|| tmp.path().join("probe")),
            tmp.path().join("chosen")
        );
        // a later set replaces the earlier one (Settings → change folder)
        cache.set(tmp.path().join("chosen2"));
        assert_eq!(cache.resolve_with(|| PathBuf::from("/never")), tmp.path().join("chosen2"));
    }
```
(`tempfile` is already a dev-dependency — the existing tests in this module use `tempfile::tempdir()`.)

- [ ] **Step 2: Run — expect FAIL** (`RecordingsRootCache` not defined)

Run: `cd frontend/src-tauri && source ~/.cargo/env && cargo test --features metal recording_preferences 2>&1 | tail -5`

- [ ] **Step 3: Implement the cache**

In `recording_preferences.rs`, above `get_default_recordings_folder`:
```rust
use std::sync::RwLock;

/// The ACTIVE recordings write root (specs/0057 Plan 2, Task 1).
///
/// Every writer (saver, import, recovery, reconcile, diarization) used to call
/// `get_default_recordings_folder()` — a filesystem probe — while `fs_guard`, folder delete
/// and "Open recordings folder" read the PERSISTED `save_folder`. The two disagreed whenever a
/// legacy/new folder appeared on disk after the preference was stored. This cache is set from
/// the persisted preference at startup and on every preference save, so writers and readers
/// share one source of truth; the probe is only the fallback when no preference was ever
/// stored (first launch, unit tests).
#[derive(Default)]
pub struct RecordingsRootCache(RwLock<Option<PathBuf>>);

impl RecordingsRootCache {
    pub fn set(&self, root: PathBuf) {
        *self.0.write().unwrap_or_else(|e| e.into_inner()) = Some(root);
    }
    pub fn resolve_with(&self, fallback: impl FnOnce() -> PathBuf) -> PathBuf {
        match self.0.read().unwrap_or_else(|e| e.into_inner()).as_ref() {
            Some(p) => p.clone(),
            None => fallback(),
        }
    }
}

static RECORDINGS_ROOT: RecordingsRootCache = RecordingsRootCache(RwLock::new(None));

/// Set the active write root (called from `init_recordings_root` and `save_recording_preferences`).
pub fn set_recordings_root(root: PathBuf) {
    RECORDINGS_ROOT.set(root);
}

/// The folder new recordings are written to. Prefer this over `get_default_recordings_folder()`.
pub fn recordings_root() -> PathBuf {
    RECORDINGS_ROOT.resolve_with(get_default_recordings_folder)
}

/// Load the persisted preference once at startup and seed the cache. Failure to load leaves
/// the probe fallback in place (logged).
pub async fn init_recordings_root<R: Runtime>(app: &AppHandle<R>) {
    match load_recording_preferences(app).await {
        Ok(prefs) => set_recordings_root(prefs.save_folder),
        Err(e) => log::warn!("recordings root: could not load preferences, using default probe: {e}"),
    }
}
```
`RwLock::new` is `const` on stable Rust ≥1.63, so the `static` initializer compiles (rust-version is 1.88).

In `save_recording_preferences`, after the successful `store.save()` (line ~224) and before `ensure_recordings_directory`, add `set_recordings_root(preferences.save_folder.clone());`.

Re-export in `audio/mod.rs:117`: `pub use recording_preferences::{get_default_recordings_folder, init_recordings_root, recordings_root, RecordingPreferences};`

- [ ] **Step 4: Seed at startup**

In `lib.rs` setup hook, right after `app_paths::init(...)` / the existing `init_bundle_identifier` call (search for `app_paths::init`), add:
```rust
            // specs/0057 Plan 2 — seed the active recordings write root from the persisted
            // preference before any writer can run (recovery reconcile spawns right after).
            {
                let handle = app.handle().clone();
                tauri::async_runtime::block_on(crate::audio::init_recordings_root(&handle));
            }
```
Confirm the setup hook is synchronous (it is `|app| { ... Ok(()) }`); `block_on` is fine there and must run BEFORE `spawn_startup_reconciliation` is called (find that call in `lib.rs` and keep the seed above it).

- [ ] **Step 5: Replace the six call sites**

Each becomes `crate::audio::recordings_root()` (or `recordings_root()` with a `use` edit):
1. `audio/recording_saver.rs:515` — `let base = crate::audio::recordings_root();` (sync, no handle needed now).
2. `audio/import.rs:329` — same.
3. `audio/recording_recovery.rs:289` and `:322` — same.
4. `audio/processing_reconcile.rs:121-123` — the fn-pointer passed into `spawn_blocking` becomes `crate::audio::recordings_root` (same signature `fn() -> PathBuf`).
5. `diarization/pipeline.rs:209-212` — replace the local `use ... get_default_recordings_folder` with `recordings_root`.

Then: `grep -rn "get_default_recordings_folder" frontend/src-tauri/src` must show only: its definition, the `Default` impl (`:67`), the `get_default_recordings_folder_path` command, the `recordings_root` fallback, and `mod.rs`'s re-export.

- [ ] **Step 6: Run tests, clippy, check**

Run: `cd frontend/src-tauri && source ~/.cargo/env && cargo test --features metal recording_preferences 2>&1 | tail -5 && cargo clippy --features metal --all-targets -- -D warnings 2>&1 | tail -2 && cargo check --features metal 2>&1 | tail -1`
Expected: 6 tests pass; clippy clean.

- [ ] **Step 7: `.env.google` secret check in both scripts**

In `frontend/dev-nixon.sh` and `frontend/upgrade-nixon.sh`, the Plan 1 warning condition `[ -f src-tauri/.env.google ] && [ -z "${NIXON_GOOGLE_CLIENT_ID:-}" ]` becomes `[ -f src-tauri/.env.google ] && { [ -z "${NIXON_GOOGLE_CLIENT_ID:-}" ] || [ -z "${NIXON_GOOGLE_CLIENT_SECRET:-}" ]; }` and the warning text names both keys. `bash -n` both.

- [ ] **Step 8: Commit**

```bash
git add frontend/src-tauri/src frontend/dev-nixon.sh frontend/upgrade-nixon.sh
git commit -m "fix(0057): recordings write root reads the persisted save_folder; .env.google secret check"
```

---

### Task 2: Rail height reservation

**Files:**
- Modify: `frontend/src/app/globals.css`, `frontend/src/components/MainContent/index.tsx`, the ten page roots listed in the file map.

**Interfaces:**
- Produces: CSS variable `--rail-h: 2.75rem` on `:root`; utility class `.h-page` = `height: calc(100vh - var(--rail-h))`; `MainContent` gets `pb-[var(--rail-h)]`. Task 6 mounts a `fixed bottom-0` rail of exactly `var(--rail-h)`.

- [ ] **Step 1: Write the failing test**

`frontend/src/components/__tests__/MainContent.test.tsx`:
```tsx
import { describe, it, expect, vi } from 'vitest';
import { render } from '@testing-library/react';
import MainContent from '@/components/MainContent';

vi.mock('@/components/Sidebar/SidebarProvider', () => ({
  useSidebar: () => ({ isCollapsed: true }),
}));

// specs/0057 Plan 2 Task 2 — the persistent transport rail is fixed to the bottom edge, so
// the main column reserves its height; otherwise page footers hide behind the rail.
describe('MainContent', () => {
  it('reserves the rail height at the bottom', () => {
    const { container } = render(<MainContent><div>x</div></MainContent>);
    const main = container.querySelector('main')!;
    expect(main.className).toContain('pb-[var(--rail-h)]');
  });
});
```
Check how `MainContent` imports `useSidebar` (it imports from `../Sidebar/SidebarProvider` or `@/components/Sidebar/SidebarProvider`); mock the exact module specifier it uses.

- [ ] **Step 2: Run — expect FAIL** (`pnpm test -- MainContent`)

- [ ] **Step 3: Implement**

`globals.css`, inside the `:root` token block (both themes share it, so put it in `:root` only): `--rail-h: 2.75rem; /* transport rail height (specs/0057 Plan 2) */`. In `@layer components` add:
```css
  /* Page root height under the persistent transport rail. Replaces bare `h-screen` on page
     roots so nothing hides behind the rail. */
  .h-page {
    height: calc(100vh - var(--rail-h));
  }
```
`MainContent/index.tsx`: className becomes `` `min-w-0 flex-1 pb-[var(--rail-h)] transition-all duration-300 ${isCollapsed ? 'ml-16' : 'ml-64'}` ``.

Page roots: in each file listed in the file map replace the page-root `h-screen` with `h-page` (only the ROOT container of the page, e.g. `"flex h-screen flex-col bg-background"` → `"flex h-page flex-col bg-background"`). Centered loading/empty states (`meeting-details/page.tsx:480-541`, `ask/page.tsx:678`, `saved-question` and `person-details` loading divs, `app/page.tsx:273` Suspense fallback) also switch to `h-page` so the spinner is centered in the visible area. Then `grep -rn "h-screen" frontend/src/app` must return nothing.

- [ ] **Step 4: Verify**

Run: `cd frontend && pnpm test -- MainContent && pnpm lint && npx tsc --noEmit`; launch `./dev-nixon.sh` briefly — pages render with a 44px empty band at the bottom (the rail arrives in Task 6).

- [ ] **Step 5: Commit** — `git commit -am "feat(0057): reserve transport-rail height on every page (--rail-h, .h-page)"` (add the new test file first).

---

### Task 3: Transport primitives — elapsed formatter, tape counter, transport key, lamp

**Files:**
- Create: `frontend/src/lib/transport/format-elapsed.ts`, `frontend/src/lib/transport/__tests__/format-elapsed.test.ts`, `frontend/src/components/Transport/TapeCounter.tsx`, `TransportKey.tsx`, `LampDot.tsx`, `frontend/src/components/Transport/__tests__/TapeCounter.test.tsx`, `TransportKey.test.tsx`

**Interfaces:**
- `formatElapsedHms(totalSeconds: number): string` → always `h:mm:ss` (`0:00:00`, `1:04:12`), negative/NaN → `0:00:00`.
- `<TapeCounter seconds={number} size="sm"|"xs" tone="normal"|"amber"|"dim" />` — digits in wells; `aria-label="Elapsed time"`; text content equals `formatElapsedHms(seconds)`.
- `<TransportKey fn="rec"|"hold"|"stop" lit={boolean} dim={boolean} disabled={boolean} onClick aria-label legend />` — 44×34 key with lamp bar, glyph, legend.
- `<LampDot tone="off"|"amber"|"red"|"green" label={string} />` — 8px lamp with `role="status"` and `aria-label={label}` when tone ≠ off.

- [ ] **Step 1: Write failing tests**

`lib/transport/__tests__/format-elapsed.test.ts`:
```ts
import { describe, it, expect } from 'vitest';
import { formatElapsedHms } from '@/lib/transport/format-elapsed';

// specs/0057 §3.3 — a counter does not hide its digits: always h:mm:ss with leading zeros.
describe('formatElapsedHms', () => {
  it('formats zero and sub-minute values', () => {
    expect(formatElapsedHms(0)).toBe('0:00:00');
    expect(formatElapsedHms(7)).toBe('0:00:07');
  });
  it('formats minutes and hours', () => {
    expect(formatElapsedHms(252)).toBe('0:04:12');
    expect(formatElapsedHms(3852)).toBe('1:04:12');
    expect(formatElapsedHms(36000)).toBe('10:00:00');
  });
  it('floors fractions and clamps garbage', () => {
    expect(formatElapsedHms(59.9)).toBe('0:00:59');
    expect(formatElapsedHms(-5)).toBe('0:00:00');
    expect(formatElapsedHms(Number.NaN)).toBe('0:00:00');
  });
});
```
`components/Transport/__tests__/TapeCounter.test.tsx`:
```tsx
import { describe, it, expect } from 'vitest';
import { render, screen } from '@testing-library/react';
import { TapeCounter } from '@/components/Transport/TapeCounter';

describe('TapeCounter', () => {
  it('renders every digit in its own well and exposes the time as text', () => {
    render(<TapeCounter seconds={252} size="sm" />);
    const el = screen.getByLabelText('Elapsed time');
    expect(el.textContent?.replace(/\s/g, '')).toBe('0:04:12');
    expect(el.querySelectorAll('[data-digit]').length).toBe(6);
  });
  it('applies the amber tone when frozen on hold', () => {
    render(<TapeCounter seconds={1} size="sm" tone="amber" />);
    expect(screen.getByLabelText('Elapsed time').dataset.tone).toBe('amber');
  });
});
```
`components/Transport/__tests__/TransportKey.test.tsx`:
```tsx
import { describe, it, expect, vi } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';
import { TransportKey } from '@/components/Transport/TransportKey';

describe('TransportKey', () => {
  it('is a button with the given accessible name and reflects lit state', () => {
    const onClick = vi.fn();
    render(<TransportKey fn="rec" lit legend="REC" aria-label="Recording" onClick={onClick} />);
    const btn = screen.getByRole('button', { name: 'Recording' });
    expect(btn.getAttribute('aria-pressed')).toBe('true');
    fireEvent.click(btn);
    expect(onClick).toHaveBeenCalledTimes(1);
  });
  it('does not fire when disabled', () => {
    const onClick = vi.fn();
    render(<TransportKey fn="hold" legend="HOLD" aria-label="Pause recording" disabled onClick={onClick} />);
    fireEvent.click(screen.getByRole('button', { name: 'Pause recording' }));
    expect(onClick).not.toHaveBeenCalled();
  });
});
```

- [ ] **Step 2: Run — expect FAIL** (`pnpm test -- transport Transport`)

- [ ] **Step 3: Implement**

`lib/transport/format-elapsed.ts`:
```ts
/** specs/0057 §3.3 — tape-counter format. Always h:mm:ss; a counter does not hide its digits. */
export function formatElapsedHms(totalSeconds: number): string {
  const s = Number.isFinite(totalSeconds) && totalSeconds > 0 ? Math.floor(totalSeconds) : 0;
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const sec = s % 60;
  return `${h}:${String(m).padStart(2, '0')}:${String(sec).padStart(2, '0')}`;
}
```
`components/Transport/TapeCounter.tsx`:
```tsx
'use client';

import React from 'react';
import { cn } from '@/lib/utils';
import { formatElapsedHms } from '@/lib/transport/format-elapsed';

// specs/0057 §3.3 — mechanical roller counter: each digit sits in a recessed well and rolls
// vertically (110 ms) when it changes. Not 7-segment, not Nixie.
type Size = 'sm' | 'xs';
type Tone = 'normal' | 'amber' | 'dim';

const WELL: Record<Size, string> = {
  sm: 'w-3 h-[18px] text-[13px]',
  xs: 'w-2.5 h-[15px] text-[11px]',
};
const COLON: Record<Size, string> = { sm: 'text-[12px]', xs: 'text-[10px]' };
const TONE: Record<Tone, string> = {
  normal: 'text-foreground',
  amber: 'text-brand',
  dim: 'text-muted-foreground',
};

function Digit({ value, size, tone }: { value: string; size: Size; tone: Tone }) {
  // The roll: a 10-row strip translated by -value*100%; transition on transform.
  const n = Number(value);
  return (
    <span
      data-digit={value}
      className={cn(
        'relative inline-flex overflow-hidden rounded-[2px] bg-well font-sans font-semibold tabular-nums tracking-[-0.02em]',
        'shadow-[inset_0_1px_2px_rgba(0,0,0,0.6),inset_0_-1px_0_hsl(var(--bevel-hi))]',
        WELL[size],
        TONE[tone],
      )}
    >
      <span
        aria-hidden
        className="flex flex-col items-center transition-transform duration-[110ms] ease-out motion-reduce:transition-none"
        style={{ transform: `translateY(-${n * 10}%)` }}
      >
        {Array.from({ length: 10 }, (_, i) => (
          <span key={i} className="flex h-full w-full items-center justify-center leading-none" style={{ height: '100%' }}>
            {i}
          </span>
        ))}
      </span>
      <span className="sr-only">{value}</span>
    </span>
  );
}

export function TapeCounter({
  seconds,
  size = 'sm',
  tone = 'normal',
  className,
}: {
  seconds: number;
  size?: Size;
  tone?: Tone;
  className?: string;
}) {
  const text = formatElapsedHms(seconds);
  return (
    <span
      role="timer"
      aria-label="Elapsed time"
      data-tone={tone}
      className={cn('inline-flex items-center gap-0.5', className)}
    >
      {text.split('').map((ch, i) =>
        ch === ':' ? (
          <span key={i} className={cn('px-px font-semibold text-engrave', COLON[size])} aria-hidden>
            :
          </span>
        ) : (
          <Digit key={i} value={ch} size={size} tone={tone} />
        ),
      )}
    </span>
  );
}
```
Note on the roll: each `Digit` well has a fixed height and the strip is 10× that height (each row `height: 100%` of the well via the flex-col parent being `1000%` tall). Set the strip's height explicitly: give the inner strip `style={{ transform: ..., height: '1000%' }}` and each row `h-[10%]`. The test only checks text and count; get the visual right by eye in Step 4.

`components/Transport/LampDot.tsx`:
```tsx
import React from 'react';
import { cn } from '@/lib/utils';

// specs/0057 §2 — an indicator lamp: flat token fill + a tight halo. No blur filters.
// Lamp attack 120 ms / decay 400 ms is done with a transition on background/box-shadow.
export type LampTone = 'off' | 'amber' | 'red' | 'green';

const TONE: Record<LampTone, string> = {
  off: 'bg-border shadow-[inset_0_1px_1px_rgba(0,0,0,0.5)] duration-[400ms]',
  amber: 'bg-brand shadow-[0_0_0_1px_hsl(var(--brand)/0.25),0_0_8px_-1px_hsl(var(--brand)/0.55)] duration-[120ms]',
  red: 'bg-record shadow-[0_0_0_1px_hsl(var(--record)/0.25),0_0_8px_-1px_hsl(var(--record)/0.55)] duration-[120ms]',
  green: 'bg-success shadow-[0_0_0_1px_hsl(var(--success)/0.25),0_0_8px_-1px_hsl(var(--success)/0.55)] duration-[120ms]',
};

export function LampDot({ tone, label, className }: { tone: LampTone; label: string; className?: string }) {
  return (
    <span
      role="status"
      aria-label={tone === 'off' ? `${label} off` : label}
      data-tone={tone}
      className={cn('inline-block h-2 w-2 flex-none rounded-full transition-[background-color,box-shadow] ease-out', TONE[tone], className)}
    />
  );
}
```
`components/Transport/TransportKey.tsx`:
```tsx
'use client';

import React from 'react';
import { cn } from '@/lib/utils';

// specs/0057 §3.1 — an illuminated transport key: 44×34, 2px radius, 1px bevel, engraved
// 9px caps legend, and a 3px lamp bar across the top that lights in the key's own color.
// REC lamp = record, HOLD lamp = brand, STOP is never lit.
export type TransportFn = 'rec' | 'hold' | 'stop';

const GLYPH: Record<TransportFn, React.ReactNode> = {
  rec: <circle cx="6" cy="6" r="5" />,
  hold: (
    <>
      <rect x="1.5" y="1" width="3.2" height="10" />
      <rect x="7.3" y="1" width="3.2" height="10" />
    </>
  ),
  stop: <rect x="1.5" y="1.5" width="9" height="9" />,
};

const LIT_BAR: Record<TransportFn, string> = {
  rec: 'bg-record shadow-[0_0_0_1px_hsl(var(--record)/0.25),0_0_8px_-1px_hsl(var(--record)/0.7)]',
  hold: 'bg-brand shadow-[0_0_0_1px_hsl(var(--brand)/0.25),0_0_8px_-1px_hsl(var(--brand)/0.7)]',
  stop: '',
};
const LIT_GLYPH: Record<TransportFn, string> = { rec: 'fill-record', hold: 'fill-brand', stop: 'fill-engrave' };

export interface TransportKeyProps extends Omit<React.ButtonHTMLAttributes<HTMLButtonElement>, 'children'> {
  fn: TransportFn;
  legend: string;
  lit?: boolean;
  /** lit but dimmed to 55% (REC while on HOLD) */
  dim?: boolean;
}

export function TransportKey({ fn, legend, lit = false, dim = false, disabled, className, ...rest }: TransportKeyProps) {
  return (
    <button
      type="button"
      aria-pressed={lit}
      disabled={disabled}
      className={cn(
        'relative flex h-[34px] w-11 flex-none flex-col items-center justify-end gap-0.5 rounded-[2px] pb-1',
        'bg-key shadow-[inset_0_1px_0_hsl(var(--bevel-hi)),inset_0_-1px_0_hsl(var(--bevel-lo)),0_1px_0_rgba(0,0,0,0.4)]',
        'transition-transform duration-[60ms] ease-[cubic-bezier(.2,0,0,1)] active:translate-y-px',
        'focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-panel',
        lit && 'shadow-[inset_0_1px_0_hsl(var(--bevel-lo)),inset_0_-1px_0_hsl(var(--bevel-hi))]',
        disabled && 'opacity-45',
        className,
      )}
      {...rest}
    >
      <span
        aria-hidden
        className={cn(
          'absolute left-2 right-2 top-[3px] h-[3px] rounded-[1px] transition-[background-color,box-shadow] ease-out',
          lit ? cn(LIT_BAR[fn], 'duration-[120ms]') : 'bg-border duration-[400ms]',
          lit && dim && 'opacity-55',
        )}
      />
      <svg aria-hidden viewBox="0 0 12 12" className={cn('h-3 w-3', lit ? LIT_GLYPH[fn] : 'fill-engrave', lit && dim && 'opacity-55')}>
        {GLYPH[fn]}
      </svg>
      <span className={cn('text-[9px] font-semibold tracking-[0.12em]', lit ? 'text-foreground' : 'text-engrave')}>{legend}</span>
    </button>
  );
}
```
Tailwind needs `fill-record`, `fill-brand`, `fill-engrave`: these come from the color map automatically (`fill-*` uses `theme.colors`), and `opacity-45`/`opacity-55` need arbitrary values `opacity-[0.45]`/`opacity-[0.55]` if not in the default scale (Tailwind 3 has 0,5,10,…,100 in steps of 5 — 45 and 55 exist since v3.3; verify with a build, else use arbitrary).

- [ ] **Step 4: Run — expect PASS**; then look

Run: `cd frontend && pnpm test -- transport Transport 2>&1 | tail -4`. Add a temporary render of the three keys and a counter to `AppearanceSettings` (do NOT commit that) and `./dev-nixon.sh` to eyeball the key bevels and the digit roll in both themes; remove the temporary render.

- [ ] **Step 5: Commit** — `git add frontend/src/lib/transport frontend/src/components/Transport && git commit -m "feat(0057): transport primitives — tape counter, transport key, lamp"`

---

### Task 4: Level feed, ladder, VU ballistics, VU meter, reels

**Files:**
- Create: `frontend/src/lib/transport/vu-ballistics.ts`, `lib/transport/ladder.ts`, `lib/transport/__tests__/vu-ballistics.test.ts`, `lib/transport/__tests__/ladder.test.ts`, `frontend/src/hooks/useRecordingLevel.ts`, `hooks/useMicGate.ts`, `components/Transport/LevelLadder.tsx`, `components/Transport/VuMeter.tsx`, `components/Transport/Reels.tsx`, `components/Transport/__tests__/Reels.test.tsx`

**Interfaces:**
- `createVuIntegrator(opts?: { settleMs?: number; overshoot?: number }): { step(targetDb: number, dtMs: number): number; value(): number; reset(): void }` — second-order integrator; reaches 99% of a step in `settleMs` (300 default) with ≤1.5% overshoot.
- `rmsToVu(rms: number): number` → dB in [-20, +3] (`20·log10(rms)` clamped; rms ≤ 0 → -20).
- `vuToArcFraction(db: number): number` → 0..1 along the scale, 0 VU at 0.72.
- `ladderSegments(level01: number, count = 10): Array<'off'|'g'|'a'|'r'>` — segments lit up to `round(level*count)`; last two are `r` (over), the two before `a`, rest `g`.
- `useRecordingLevel(enabled: boolean): { rms: number; peak: number; peakLatched: boolean }` — listens to `recording-level`; `peakLatched` true for 800 ms after `peak > 0.98`; decays to 0 after 500 ms of silence.
- `useMicGate(): boolean` — listens to `zoom-mute-changed` `{muted}`; false initially.
- `<LevelLadder level={0..1} count={10} active={boolean} />`, `<VuMeter db={number} active={boolean} label="CH 1 · Mic" />`, `<Reels state="idle"|"recording"|"paused"|"finalizing" size={22|50} />`.

- [ ] **Step 1: Write failing tests**

`lib/transport/__tests__/vu-ballistics.test.ts`:
```ts
import { describe, it, expect } from 'vitest';
import { createVuIntegrator, rmsToVu, vuToArcFraction } from '@/lib/transport/vu-ballistics';

// specs/0057 §3.2 — true VU ballistics: 300 ms to 99% of a step input, symmetric return,
// ≤1.5% overshoot. Stepped at the 12.5 Hz feed and at 60 fps; both must converge.
describe('createVuIntegrator', () => {
  function runStep(dtMs: number, totalMs: number) {
    const vu = createVuIntegrator();
    let t = 0;
    let max = -Infinity;
    let at99: number | null = null;
    while (t < totalMs) {
      const v = vu.step(0, dtMs); // target 0 dB from rest at -20 dB
      max = Math.max(max, v);
      if (at99 === null && v >= -0.2) at99 = t; // 99% of a 20 dB step
      t += dtMs;
    }
    return { max, at99, final: vu.value() };
  }
  it('reaches 99% of a step within ~300 ms at 60 fps', () => {
    const { at99, max, final } = runStep(16.7, 1000);
    expect(at99).not.toBeNull();
    expect(at99!).toBeLessThanOrEqual(340);
    expect(at99!).toBeGreaterThanOrEqual(220);
    expect(max).toBeLessThanOrEqual(0.3); // ≤1.5% of 20 dB overshoot
    expect(final).toBeCloseTo(0, 1);
  });
  it('converges without blowing up at the 80 ms feed cadence', () => {
    const { max, final } = runStep(80, 1000);
    expect(max).toBeLessThanOrEqual(0.5);
    expect(final).toBeCloseTo(0, 1);
  });
  it('returns symmetrically', () => {
    const vu = createVuIntegrator();
    for (let i = 0; i < 60; i++) vu.step(0, 16.7);
    let t = 0;
    let at99: number | null = null;
    while (t < 1000) {
      const v = vu.step(-20, 16.7);
      if (at99 === null && v <= -19.8) at99 = t;
      t += 16.7;
    }
    expect(at99!).toBeLessThanOrEqual(340);
  });
});

describe('rmsToVu / vuToArcFraction', () => {
  it('maps rms to the -20..+3 VU face', () => {
    expect(rmsToVu(0)).toBe(-20);
    expect(rmsToVu(1)).toBe(0);
    expect(rmsToVu(0.1)).toBeCloseTo(-20, 5);
    expect(rmsToVu(2)).toBe(3); // clamp over
  });
  it('puts 0 VU at 72% of arc travel and -20 at 0', () => {
    expect(vuToArcFraction(-20)).toBe(0);
    expect(vuToArcFraction(0)).toBeCloseTo(0.72, 5);
    expect(vuToArcFraction(3)).toBe(1);
  });
});
```
`lib/transport/__tests__/ladder.test.ts`:
```ts
import { describe, it, expect } from 'vitest';
import { ladderSegments } from '@/lib/transport/ladder';

describe('ladderSegments', () => {
  it('lights proportionally with green, then amber, then red at the top', () => {
    expect(ladderSegments(0)).toEqual(Array(10).fill('off'));
    expect(ladderSegments(0.5)).toEqual(['g', 'g', 'g', 'g', 'g', 'off', 'off', 'off', 'off', 'off']);
    expect(ladderSegments(1)).toEqual(['g', 'g', 'g', 'g', 'g', 'g', 'a', 'a', 'r', 'r']);
  });
  it('clamps', () => {
    expect(ladderSegments(-1)[0]).toBe('off');
    expect(ladderSegments(9)[9]).toBe('r');
  });
});
```
`components/Transport/__tests__/Reels.test.tsx`:
```tsx
import { describe, it, expect } from 'vitest';
import { render } from '@testing-library/react';
import { Reels } from '@/components/Transport/Reels';

describe('Reels', () => {
  it('spins only while recording and stops dead on hold', () => {
    const { container, rerender } = render(<Reels state="recording" />);
    expect(container.firstElementChild!.getAttribute('data-state')).toBe('recording');
    expect(container.querySelector('[data-hub]')!.className).toContain('animate-reel');
    rerender(<Reels state="paused" />);
    expect(container.querySelector('[data-hub]')!.className).not.toContain('animate-reel');
  });
});
```

- [ ] **Step 2: Run — expect FAIL**

- [ ] **Step 3: Implement the helpers**

`lib/transport/vu-ballistics.ts`:
```ts
/**
 * specs/0057 §3.2 — VU meter ballistics.
 *
 * A critically-damped-ish second-order system: the needle position `x` chases the target
 * with velocity `v`; `omega` is chosen so a step reaches 99% in `settleMs`. Slight
 * under-damping (zeta ≈ 0.85) gives the classic ~1–1.5% overshoot. Stepped with the real
 * elapsed dt so it converges identically at the 80 ms event cadence and at 60 fps.
 */
export interface VuIntegrator {
  step(targetDb: number, dtMs: number): number;
  value(): number;
  reset(db?: number): void;
}

export const VU_MIN_DB = -20;
export const VU_MAX_DB = 3;

export function createVuIntegrator(opts: { settleMs?: number; zeta?: number } = {}): VuIntegrator {
  const settle = opts.settleMs ?? 300;
  const zeta = opts.zeta ?? 0.85;
  // For a 2nd-order system, 1% settling ≈ 4.6 / (zeta·omega) ⇒ omega = 4.6 / (zeta·T)
  const omega = 4.6 / (zeta * (settle / 1000));
  let x = VU_MIN_DB;
  let v = 0;
  return {
    step(target, dtMs) {
      // Sub-step so large dt (80 ms feed) stays stable: cap each integration step at 8 ms.
      let remaining = Math.max(0, dtMs) / 1000;
      while (remaining > 0) {
        const h = Math.min(remaining, 0.008);
        const a = omega * omega * (target - x) - 2 * zeta * omega * v;
        v += a * h;
        x += v * h;
        remaining -= h;
      }
      x = Math.min(VU_MAX_DB + 0.5, Math.max(VU_MIN_DB - 0.5, x));
      return x;
    },
    value: () => x,
    reset(db = VU_MIN_DB) {
      x = db;
      v = 0;
    },
  };
}

/** 20·log10(rms) onto the classic face: -20 … +3 VU. */
export function rmsToVu(rms: number): number {
  if (!(rms > 0)) return VU_MIN_DB;
  const db = 20 * Math.log10(rms);
  return Math.min(VU_MAX_DB, Math.max(VU_MIN_DB, db));
}

/** Arc travel 0..1 with 0 VU at 72% (the red zone is the last 28%). Piecewise-linear. */
export function vuToArcFraction(db: number): number {
  const d = Math.min(VU_MAX_DB, Math.max(VU_MIN_DB, db));
  if (d <= 0) return ((d - VU_MIN_DB) / (0 - VU_MIN_DB)) * 0.72;
  return 0.72 + (d / VU_MAX_DB) * 0.28;
}
```
If the tests' timing bounds fail by a little, tune `zeta` (0.8–0.9) and the `4.6` constant until the 60 fps run hits 99% between 220–340 ms with overshoot ≤0.3 dB; record the final constants in the report. Do not loosen the tests beyond the spec's 300 ms / 1.5%.

`lib/transport/ladder.ts`:
```ts
export type LadderSeg = 'off' | 'g' | 'a' | 'r';

/** specs/0057 decision 6 — the compact level meter: N segments, last two red, two before amber. */
export function ladderSegments(level01: number, count = 10): LadderSeg[] {
  const lit = Math.round(Math.min(1, Math.max(0, level01)) * count);
  return Array.from({ length: count }, (_, i) => {
    if (i >= lit) return 'off';
    if (i >= count - 2) return 'r';
    if (i >= count - 4) return 'a';
    return 'g';
  });
}
```

- [ ] **Step 4: Hooks**

`hooks/useRecordingLevel.ts`:
```ts
'use client';

import { useEffect, useRef, useState } from 'react';
import { safeListen } from '@/lib/safe-listen';

// specs/0057 §3.2 — the backend emits `recording-level` {rms, peak} at ~12.5 Hz
// (audio/pipeline.rs LIVE_SPECTRUM_EMIT_INTERVAL). Nothing consumed it before Plan 2.
const SILENCE_DECAY_MS = 500;
const PEAK_LATCH_MS = 800;

export interface RecordingLevel {
  rms: number;
  peak: number;
  peakLatched: boolean;
}

export function useRecordingLevel(enabled: boolean): RecordingLevel {
  const [level, setLevel] = useState<RecordingLevel>({ rms: 0, peak: 0, peakLatched: false });
  const decayTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const latchTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    if (!enabled) {
      setLevel({ rms: 0, peak: 0, peakLatched: false });
      return;
    }
    let disposed = false;
    const disposeP = safeListen<{ rms: number; peak: number }>('recording-level', (event) => {
      if (disposed) return;
      const rms = Number(event.payload?.rms) || 0;
      const peak = Number(event.payload?.peak) || 0;
      setLevel((prev) => ({ rms, peak, peakLatched: prev.peakLatched || peak > 0.98 }));
      if (peak > 0.98) {
        if (latchTimer.current) clearTimeout(latchTimer.current);
        latchTimer.current = setTimeout(() => setLevel((p) => ({ ...p, peakLatched: false })), PEAK_LATCH_MS);
      }
      if (decayTimer.current) clearTimeout(decayTimer.current);
      decayTimer.current = setTimeout(() => setLevel((p) => ({ ...p, rms: 0, peak: 0 })), SILENCE_DECAY_MS);
    });
    return () => {
      disposed = true;
      disposeP.then((un) => un && un()).catch(() => {});
      if (decayTimer.current) clearTimeout(decayTimer.current);
      if (latchTimer.current) clearTimeout(latchTimer.current);
    };
  }, [enabled]);

  return level;
}
```
Check `safeListen`'s real return type in `lib/safe-listen.ts` (the existing `useRecordingWaveform.ts:44` shows the usage pattern) and match it.

`hooks/useMicGate.ts`:
```ts
'use client';

import { useEffect, useState } from 'react';
import { safeListen } from '@/lib/safe-listen';

// specs/0049 + 0057 §3.1 — the Zoom mute gate already emits `zoom-mute-changed` {muted}
// (src-tauri/src/zoom/mute_monitor.rs EVENT_MUTE_CHANGED); no frontend listened before Plan 2.
export function useMicGate(): boolean {
  const [muted, setMuted] = useState(false);
  useEffect(() => {
    const disposeP = safeListen<{ muted: boolean }>('zoom-mute-changed', (e) => setMuted(Boolean(e.payload?.muted)));
    return () => {
      disposeP.then((un) => un && un()).catch(() => {});
    };
  }, []);
  return muted;
}
```

- [ ] **Step 5: Components**

`components/Transport/LevelLadder.tsx`:
```tsx
import React from 'react';
import { cn } from '@/lib/utils';
import { ladderSegments } from '@/lib/transport/ladder';

const SEG: Record<'off' | 'g' | 'a' | 'r', string> = {
  off: 'bg-border/70',
  g: 'bg-success',
  a: 'bg-brand',
  r: 'bg-record',
};

/** specs/0057 decision 6 — 10-segment level ladder for compact spots (rail, device picker). */
export function LevelLadder({ level, count = 10, active = true, className }: { level: number; count?: number; active?: boolean; className?: string }) {
  const segs = ladderSegments(active ? level : 0, count);
  return (
    <span aria-hidden className={cn('inline-flex h-3.5 items-end gap-0.5', className)}>
      {segs.map((s, i) => (
        <i key={i} className={cn('block h-full w-1.5 shadow-[inset_0_1px_0_rgba(255,255,255,0.08)] transition-colors duration-[60ms]', SEG[s])} />
      ))}
    </span>
  );
}
```
`components/Transport/VuMeter.tsx` — the SVG face from the mockup, needle rotated by arc fraction. The scale spans -50° … +50° around a pivot at (90, 95) in a 180×78 viewBox; the arc from -20 to 0 VU is at radius 78; the red zone is 0 … +3:
```tsx
'use client';

import React, { useEffect, useRef, useState } from 'react';
import { cn } from '@/lib/utils';
import { createVuIntegrator, vuToArcFraction } from '@/lib/transport/vu-ballistics';

const ANGLE_MIN = -50;
const ANGLE_MAX = 50;

/**
 * specs/0057 §3.2 — needle VU. `db` is the raw target (from rmsToVu); the needle follows it
 * through the ballistics integrator stepped on requestAnimationFrame, so the 12.5 Hz feed
 * never stutters. Under prefers-reduced-motion the needle snaps (no rAF loop).
 */
export function VuMeter({ db, active, label, className }: { db: number; active: boolean; label: string; className?: string }) {
  const integ = useRef(createVuIntegrator());
  const target = useRef(db);
  const [needleDb, setNeedleDb] = useState(-20);
  target.current = active ? db : -20;

  useEffect(() => {
    const reduced = typeof window !== 'undefined' && window.matchMedia?.('(prefers-reduced-motion: reduce)').matches;
    if (reduced) {
      setNeedleDb(target.current);
      return;
    }
    let raf = 0;
    let last = performance.now();
    const tick = (now: number) => {
      const dt = now - last;
      last = now;
      setNeedleDb(integ.current.step(target.current, dt));
      raf = requestAnimationFrame(tick);
    };
    raf = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(raf);
  }, []);

  const angle = ANGLE_MIN + vuToArcFraction(needleDb) * (ANGLE_MAX - ANGLE_MIN);
  return (
    <div className={cn('flex flex-col items-center gap-1.5', className)} role="img" aria-label={`${label} level ${Math.round(needleDb)} VU`}>
      <div className="relative h-[78px] w-[180px] overflow-hidden rounded-[2px] bg-well shadow-[inset_0_1px_2px_rgba(0,0,0,0.6),inset_0_-1px_0_hsl(var(--bevel-hi))]">
        <svg viewBox="0 0 180 78" className="block h-full w-full">
          <path d="M30.3 44.9 A78 78 0 0 1 119.2 22.7" className="stroke-engrave" strokeWidth="1" fill="none" />
          <path d="M119.2 22.7 A78 78 0 0 1 149.7 44.9" className="stroke-meter-over" strokeWidth="2.5" fill="none" />
          <g className="stroke-engrave" strokeWidth="1">
            <path d="M30.3 44.9 L34.8 48.7" /><path d="M51 27.5 L54 32.7" /><path d="M69.8 19.7 L71.4 25.5" />
            <path d="M83.2 17.3 L83.7 23.3" /><path d="M100.9 17.8 L100 23.7" /><path d="M119.2 22.7 L117 28.3" />
            <path d="M145.1 39.9 L140.9 44.1" />
          </g>
          <g className="fill-engrave font-narrow" fontSize="8" textAnchor="middle">
            <text x="22.6" y="40">-20</text><text x="46" y="20">-10</text><text x="67.2" y="12">-7</text>
            <text x="82.3" y="9">-5</text><text x="102.2" y="10">-3</text><text x="123" y="15">0</text><text x="152.2" y="35">+3</text>
          </g>
          <line x1="90" y1="95" x2="90" y2="17" className="stroke-foreground" strokeWidth="1.5" transform={`rotate(${angle} 90 95)`} />
          <text x="90" y="72" className="fill-engrave font-sans" fontSize="9" fontWeight="600" letterSpacing="1.5" textAnchor="middle">VU</text>
        </svg>
      </div>
      <span className="u-section-label">{label}</span>
    </div>
  );
}
```
`stroke-engrave`, `stroke-meter-over`, `stroke-foreground`, `fill-engrave` come from the Tailwind color map (`stroke-*`/`fill-*` utilities use `theme.colors`).

`components/Transport/Reels.tsx`:
```tsx
import React from 'react';
import { cn } from '@/lib/utils';

export type ReelState = 'idle' | 'recording' | 'paused' | 'finalizing';

/**
 * specs/0057 §3.4 — the recording indicator: two hubs joined by a tape path. Turning at
 * 0.85 rev/s while recording; stops dead on HOLD (no ease-out); 600 ms spin-down when
 * finalizing. Never blinks. Under prefers-reduced-motion the hubs stay still (the REC lamp
 * and the counter carry liveness) — done with Tailwind's motion-reduce variant.
 */
export function Reels({ state, size = 22, className }: { state: ReelState; size?: number; className?: string }) {
  const spin = state === 'recording';
  const hub = cn('origin-center', spin && 'animate-reel motion-reduce:animate-none', state === 'finalizing' && 'animate-reel-spindown motion-reduce:animate-none');
  const h = size;
  const w = Math.round(size * (50 / 22));
  return (
    <svg data-state={state} width={w} height={h} viewBox="0 0 50 22" className={cn('block', className)} aria-hidden>
      <path d="M11 20 L39 20" className="stroke-engrave" strokeWidth="1" fill="none" />
      <g data-hub className={hub} style={{ transformOrigin: '11px 11px' }}>
        <circle cx="11" cy="11" r="8.5" className="stroke-foreground" strokeWidth="1.5" fill="none" />
        <circle cx="11" cy="11" r="5" className="stroke-foreground" strokeWidth="1" fill="none" />
        <g className="stroke-foreground" strokeWidth="1.6" strokeLinecap="square">
          <path d="M11 6 L11 3.5" /><path d="M6.7 13.5 L4.5 14.7" /><path d="M15.3 13.5 L17.5 14.7" />
        </g>
      </g>
      <g className={hub} style={{ transformOrigin: '39px 11px' }}>
        <circle cx="39" cy="11" r="8.5" className="stroke-foreground" strokeWidth="1.5" fill="none" />
        <circle cx="39" cy="11" r="3.2" className="stroke-foreground" strokeWidth="1" fill="none" />
        <g className="stroke-foreground" strokeWidth="1.6" strokeLinecap="square">
          <path d="M39 8 L39 3.5" /><path d="M36.3 12.6 L32.5 14.7" /><path d="M41.7 12.6 L45.5 14.7" />
        </g>
      </g>
    </svg>
  );
}
```
Add to `tailwind.config.js` `keyframes`/`animation`:
```js
  			keyframes: {
  				/* existing accordion keyframes stay */
  				reel: { from: { transform: 'rotate(0deg)' }, to: { transform: 'rotate(360deg)' } },
  			},
  			animation: {
  				/* existing */
  				reel: 'reel 1.176s linear infinite',            // 0.85 rev/s
  				'reel-spindown': 'reel 0.6s ease-out 1',
  			},
```

- [ ] **Step 6: Run — expect PASS** (`pnpm test -- transport Transport Reels`), then `pnpm lint && npx tsc --noEmit`.

- [ ] **Step 7: Commit** — `git add -A frontend/src/lib/transport frontend/src/hooks/useRecordingLevel.ts frontend/src/hooks/useMicGate.ts frontend/src/components/Transport frontend/tailwind.config.js && git commit -m "feat(0057): VU ballistics + meter, level ladder, reels, recording-level and mic-gate hooks"`

---

### Task 5: The global queue model

**Files:**
- Create: `frontend/src/lib/transport/queue-view.ts`, `frontend/src/lib/transport/__tests__/queue-view.test.ts`

**Interfaces:**
- Consumes: `BacklogView`/`BacklogItem`/`BacklogItemStatus` from `@/lib/deferred-backlog`; `LlmActivityView`, `RunningTask`, `TaskRecord` from `@/contexts/LlmActivityProvider`.
- Produces:
  ```ts
  export type QueueStage = 'waiting' | 'transcribing' | 'diarizing' | 'summarizing' | 'llm' | 'done' | 'error';
  export interface QueueRow { id: string; title: string; stage: QueueStage; stageLabel: string; source: 'backlog' | 'llm'; error?: string | null; meetingId?: string | null }
  export interface QueueView { rows: QueueRow[]; running: QueueRow | null; nextUp: QueueRow | null; count: number; lamp: 'off' | 'amber' | 'red'; failures: number }
  export function buildQueueView(backlog: BacklogView, llm: LlmActivityView | null): QueueView
  export const QUEUE_STAGE_LABEL: Record<QueueStage, string>  // WAITING · TRANSCRIBING · SPEAKERS · SUMMARIZING · AI · DONE · RETRY
  ```
  Ordering: the active backlog item first, then running LLM tasks, then waiting backlog items, then LLM failures (history with outcome failed), then done items. `count` = rows that are not done. `lamp` = red if any error/failure, amber if anything running, else off.

- [ ] **Step 1: Write the failing test**

```ts
import { describe, it, expect } from 'vitest';
import { buildQueueView } from '@/lib/transport/queue-view';
import type { BacklogView } from '@/lib/deferred-backlog';
import type { LlmActivityView } from '@/contexts/LlmActivityProvider';

const meeting = (id: string, title: string) => ({ id, title, folderPath: '/x', transcriptCount: 3 });
const backlog = (items: BacklogView['items'], processing = false): BacklogView => ({
  items, processing,
  pendingCount: items.filter((i) => i.status === 'waiting').length,
  active: items.find((i) => ['transcribing', 'diarizing', 'summarizing'].includes(i.status)) ?? null,
  activeOrdinal: 1, total: items.length,
});
const llm = (running: LlmActivityView['running'], history: LlmActivityView['history'] = []): LlmActivityView => ({
  running, history, hasFailure: history.some((h) => h.outcome.type === 'failed'),
});

// specs/0057 decision 8 — ONE queue: deferred processing + background AI, one order, one lamp.
describe('buildQueueView', () => {
  it('is empty and unlit with nothing to do', () => {
    const v = buildQueueView(backlog([]), llm([]));
    expect(v.rows).toEqual([]);
    expect(v.count).toBe(0);
    expect(v.lamp).toBe('off');
    expect(v.running).toBeNull();
  });
  it('orders active backlog, running AI, waiting, failures, done and lights amber while running', () => {
    const v = buildQueueView(
      backlog([
        { meeting: meeting('a', 'Hiring loop debrief'), status: 'transcribing' },
        { meeting: meeting('b', 'Design review'), status: 'waiting' },
        { meeting: meeting('c', 'Old one'), status: 'done' },
      ], true),
      llm([{ id: 1, kind: 'meetingSummary', label: 'Summarizing Q3 planning', note: null, meetingId: 'q3' }]),
    );
    expect(v.rows.map((r) => r.title)).toEqual(['Hiring loop debrief', 'Summarizing Q3 planning', 'Design review', 'Old one']);
    expect(v.rows.map((r) => r.stage)).toEqual(['transcribing', 'llm', 'waiting', 'done']);
    expect(v.running?.id).toBe('backlog:a');
    expect(v.nextUp?.title).toBe('Summarizing Q3 planning');
    expect(v.count).toBe(3);
    expect(v.lamp).toBe('amber');
  });
  it('lights red and counts failures when an AI task failed or a backlog item errored', () => {
    const v = buildQueueView(
      backlog([{ meeting: meeting('a', 'X'), status: 'error' }]),
      llm([], [{ id: 9, kind: 'actionItems', label: 'Extracting tasks — X', error: 'Ollama unreachable', meetingId: 'a', outcome: { type: 'failed', error: 'Ollama unreachable' } }]),
    );
    expect(v.lamp).toBe('red');
    expect(v.failures).toBe(2);
    expect(v.rows.map((r) => r.stage)).toEqual(['error', 'error']);
    expect(v.rows[1].error).toBe('Ollama unreachable');
  });
  it('ignores skipped and successful AI history', () => {
    const v = buildQueueView(backlog([]), llm([], [
      { id: 1, kind: 'prepBrief', label: 'ok', error: null, meetingId: null, outcome: { type: 'success' } },
      { id: 2, kind: 'prepBrief', label: 'skip', error: null, meetingId: null, outcome: { type: 'skipped', reason: 'no transcript' } },
    ]));
    expect(v.rows).toEqual([]);
    expect(v.lamp).toBe('off');
  });
  it('tolerates a null LLM view (provider not mounted)', () => {
    const v = buildQueueView(backlog([{ meeting: meeting('a', 'X'), status: 'waiting' }]), null);
    expect(v.count).toBe(1);
  });
});
```

- [ ] **Step 2: Run — expect FAIL**

- [ ] **Step 3: Implement**

```ts
import type { BacklogItem, BacklogItemStatus, BacklogView } from '@/lib/deferred-backlog';
import type { LlmActivityView } from '@/contexts/LlmActivityProvider';

export type QueueStage = 'waiting' | 'transcribing' | 'diarizing' | 'summarizing' | 'llm' | 'done' | 'error';

export interface QueueRow {
  id: string;
  title: string;
  stage: QueueStage;
  stageLabel: string;
  source: 'backlog' | 'llm';
  error?: string | null;
  meetingId?: string | null;
}

export interface QueueView {
  rows: QueueRow[];
  running: QueueRow | null;
  nextUp: QueueRow | null;
  count: number;
  lamp: 'off' | 'amber' | 'red';
  failures: number;
}

/** Engraved caps for the rail/panel (specs/0057 §3.6). */
export const QUEUE_STAGE_LABEL: Record<QueueStage, string> = {
  waiting: 'Waiting',
  transcribing: 'Transcribing',
  diarizing: 'Speakers',
  summarizing: 'Summarizing',
  llm: 'AI',
  done: 'Done',
  error: 'Retry',
};

const ACTIVE: BacklogItemStatus[] = ['transcribing', 'diarizing', 'summarizing'];

function backlogRow(item: BacklogItem): QueueRow {
  const stage = item.status as QueueStage;
  return {
    id: `backlog:${item.meeting.id}`,
    title: item.meeting.title,
    stage,
    stageLabel: QUEUE_STAGE_LABEL[stage],
    source: 'backlog',
    meetingId: item.meeting.id,
  };
}

/**
 * specs/0057 decision 8 — merge the deferred backlog and background LLM activity into ONE
 * ordered queue: active backlog item, running AI tasks, waiting items, failures, done.
 */
export function buildQueueView(backlog: BacklogView, llm: LlmActivityView | null): QueueView {
  const active = backlog.items.filter((i) => ACTIVE.includes(i.status)).map(backlogRow);
  const running = (llm?.running ?? []).map<QueueRow>((t) => ({
    id: `llm:${t.id}`,
    title: t.label,
    stage: 'llm',
    stageLabel: QUEUE_STAGE_LABEL.llm,
    source: 'llm',
    meetingId: t.meetingId,
  }));
  const waiting = backlog.items.filter((i) => i.status === 'waiting').map(backlogRow);
  const backlogErrors = backlog.items.filter((i) => i.status === 'error').map(backlogRow);
  const llmFailures = (llm?.history ?? [])
    .filter((h) => h.outcome.type === 'failed')
    .map<QueueRow>((h) => ({
      id: `llm:${h.id}`,
      title: h.label,
      stage: 'error',
      stageLabel: QUEUE_STAGE_LABEL.error,
      source: 'llm',
      error: h.outcome.type === 'failed' ? h.outcome.error : null,
      meetingId: h.meetingId,
    }));
  const done = backlog.items.filter((i) => i.status === 'done').map(backlogRow);

  const rows = [...active, ...running, ...waiting, ...backlogErrors, ...llmFailures, ...done];
  const failures = backlogErrors.length + llmFailures.length;
  const runningRow = active[0] ?? running[0] ?? null;
  const nextUp = rows.find((r) => r !== runningRow && r.stage !== 'done' && r.stage !== 'error') ?? null;
  const lamp = failures > 0 ? 'red' : runningRow ? 'amber' : 'off';
  return { rows, running: runningRow, nextUp, count: rows.filter((r) => r.stage !== 'done').length, lamp, failures };
}
```

- [ ] **Step 4: Run — expect PASS**; commit — `git add frontend/src/lib/transport && git commit -m "feat(0057): global queue view merges deferred backlog and LLM activity"`

---

### Task 6: The transport rail — mount, status zone, keys, queue indicator + panel

**Files:**
- Create: `frontend/src/components/Transport/TransportRail.tsx`, `TransportStatus.tsx`, `QueueIndicator.tsx`, `QueuePanel.tsx`, `components/Transport/__tests__/TransportRail.test.tsx`
- Modify: `frontend/src/app/layout.tsx` (mount; providers), delete `components/GlobalRecordingBar.tsx`, `components/DeferredBacklog/DeferredBacklogIndicator.tsx`, `components/LlmActivity/LlmActivityRow.tsx` + `components/LlmActivity/__tests__/LlmActivityRow.test.tsx`; `components/Sidebar/index.tsx:197` (remove `<LlmActivityRow />` + import)

**Interfaces:**
- Consumes: `useRecordingState()` (`isRecording`, `isPaused`, `status`, `activeDuration`, `recordingDuration`, `isStopping`, `isProcessing`, `isSaving`), `useSidebar()` (`isCollapsed`, `handleRecordingToggle`, `activeRecordingMeetingId`, `currentMeeting`), `useBacklog()`, `useLlmActivity()`, `recordingService.pauseRecording/resumeRecording`, `requestFullRecordingStop`, `useRecordingLevel`, `useMicGate`, `buildQueueView`, `TransportKey`, `TapeCounter`, `Reels`, `LevelLadder`, `LampDot`, `BacklogDetailPopover` actions (`stop`, `startNow`, `dismissDone` from `useBacklog()`), `LlmActivityPopover` retry/dismiss (read its exports; reuse its per-task retry/dismiss buttons inside `QueuePanel`, or call `api_llm_activity_retry`/`api_llm_activity_dismiss` directly as `LlmActivityProvider` does).
- Produces: `<TransportRail />` rendered once in `layout.tsx` inside the post-onboarding tree; `role="region"`, `aria-label` = `"Recording in progress"` while a session is in flight else `"Transport"`. Left zone: idle → `Reels idle` + engraved "Deck ready" + dim counter `0:00:00`; recording → reels spinning, meeting title (from `useSidebar().currentMeeting?.title` when `activeRecordingMeetingId` matches, else "Recording"), counter, ladder; paused → reels stopped, amber counter, `HOLD` lit; finalizing (`isStopping || isProcessing || isSaving`) → reels spin-down, engraved "Saving". Center: three keys (`REC` aria-label `"Start recording"` when idle / `"Recording"` when in flight; `HOLD` aria-label `"Pause recording"`/`"Resume recording"`; `STOP` aria-label `"Stop recording"`). Right zone: `QueueIndicator` (engraved "Queue", count, one-line status, `LampDot`) opening `QueuePanel` (Radix Popover, `side="top"`, list of `QueueRow`s with stage caps + a per-row ladder for active stages, actions Stop / Process all / Clear finished / Retry / Dismiss).

- [ ] **Step 1: Write the failing test**

`components/Transport/__tests__/TransportRail.test.tsx`:
```tsx
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';

const { state, sidebar, backlog, llm, pauseMock, resumeMock, stopMock } = vi.hoisted(() => ({
  state: { isRecording: false, isPaused: false, isActive: false, status: 'idle', activeDuration: null as number | null, recordingDuration: null as number | null, isStopping: false, isProcessing: false, isSaving: false },
  sidebar: { isCollapsed: true, handleRecordingToggle: vi.fn(), activeRecordingMeetingId: null as string | null, currentMeeting: null as { id: string; title: string } | null },
  backlog: { view: { items: [], pendingCount: 0, processing: false, active: null, activeOrdinal: 0, total: 0 }, stop: vi.fn(), startNow: vi.fn(), dismissDone: vi.fn(), enqueueMeeting: vi.fn() },
  llm: { view: { running: [], history: [], hasFailure: false }, dismiss: vi.fn(), retry: vi.fn() },
  pauseMock: vi.fn(), resumeMock: vi.fn(), stopMock: vi.fn(),
}));
vi.mock('@/contexts/RecordingStateContext', () => ({ useRecordingState: () => state }));
vi.mock('@/components/Sidebar/SidebarProvider', () => ({ useSidebar: () => sidebar }));
vi.mock('@/contexts/DeferredBacklogProvider', () => ({ useBacklog: () => backlog }));
vi.mock('@/contexts/LlmActivityProvider', () => ({ useLlmActivity: () => llm }));
vi.mock('@/services/recordingService', () => ({ recordingService: { pauseRecording: pauseMock, resumeRecording: resumeMock } }));
vi.mock('@/lib/recording-stop', () => ({ requestFullRecordingStop: stopMock }));
vi.mock('@/hooks/useRecordingLevel', () => ({ useRecordingLevel: () => ({ rms: 0.4, peak: 0.5, peakLatched: false }) }));
vi.mock('@/hooks/useMicGate', () => ({ useMicGate: () => false }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('next/navigation', () => ({ useRouter: () => ({ push: vi.fn() }), usePathname: () => '/' }));

import { TransportRail } from '@/components/Transport/TransportRail';

beforeEach(() => {
  Object.assign(state, { isRecording: false, isPaused: false, status: 'idle', activeDuration: null, recordingDuration: null, isStopping: false, isProcessing: false, isSaving: false });
  vi.clearAllMocks();
});

// specs/0057 §3.1 state table + decision 7: one rail, one state vocabulary.
describe('TransportRail', () => {
  it('idle: Deck ready, REC starts a recording, HOLD/STOP disabled', () => {
    render(<TransportRail />);
    expect(screen.getByRole('region', { name: 'Transport' })).toBeTruthy();
    expect(screen.getByText(/deck ready/i)).toBeTruthy();
    fireEvent.click(screen.getByRole('button', { name: 'Start recording' }));
    expect(sidebar.handleRecordingToggle).toHaveBeenCalledTimes(1);
    expect(screen.getByRole('button', { name: 'Pause recording' })).toBeDisabled();
    expect(screen.getByRole('button', { name: 'Stop recording' })).toBeDisabled();
  });
  it('recording: REC lit, counter runs, HOLD pauses, STOP requests the full stop', () => {
    Object.assign(state, { isRecording: true, isActive: true, status: 'recording', activeDuration: 252 });
    render(<TransportRail />);
    expect(screen.getByRole('region', { name: 'Recording in progress' })).toBeTruthy();
    expect(screen.getByRole('button', { name: 'Recording' }).getAttribute('aria-pressed')).toBe('true');
    expect(screen.getByLabelText('Elapsed time').textContent?.replace(/\s/g, '')).toBe('0:04:12');
    fireEvent.click(screen.getByRole('button', { name: 'Pause recording' }));
    expect(pauseMock).toHaveBeenCalled();
    fireEvent.click(screen.getByRole('button', { name: 'Stop recording' }));
    expect(stopMock).toHaveBeenCalled();
  });
  it('paused: HOLD lit and labelled Resume, counter amber', () => {
    Object.assign(state, { isRecording: true, isPaused: true, isActive: true, status: 'recording', activeDuration: 10 });
    render(<TransportRail />);
    const hold = screen.getByRole('button', { name: 'Resume recording' });
    expect(hold.getAttribute('aria-pressed')).toBe('true');
    expect(screen.getByLabelText('Elapsed time').dataset.tone).toBe('amber');
    fireEvent.click(hold);
    expect(resumeMock).toHaveBeenCalled();
  });
  it('finalizing: keys disabled and Saving shown', () => {
    Object.assign(state, { isRecording: false, status: 'saving', isSaving: true, recordingDuration: 900 });
    render(<TransportRail />);
    expect(screen.getByText(/saving/i)).toBeTruthy();
    expect(screen.getByRole('button', { name: 'Stop recording' })).toBeDisabled();
  });
  it('queue: shows count and lamp, opens the panel', () => {
    backlog.view = { items: [{ meeting: { id: 'a', title: 'Hiring loop debrief', folderPath: '/x', transcriptCount: 1 }, status: 'transcribing' }], pendingCount: 0, processing: true, active: null, activeOrdinal: 1, total: 1 };
    render(<TransportRail />);
    const btn = screen.getByRole('button', { name: /queue/i });
    expect(btn.textContent).toMatch(/1/);
    expect(btn.textContent).toMatch(/transcribing/i);
    fireEvent.click(btn);
    expect(screen.getByRole('dialog', { name: /queue/i })).toBeTruthy();
    expect(screen.getByText('Hiring loop debrief')).toBeTruthy();
  });
});
```
Adjust the mocked module specifiers to the exact imports the rail uses (e.g. `recordingService` path is `@/services/recordingService`; confirm `useLlmActivity`'s real return shape has `dismiss`/`retry` or whatever `LlmActivityProvider.tsx:119` returns and mirror it).

- [ ] **Step 2: Run — expect FAIL**

- [ ] **Step 3: Implement the rail**

`components/Transport/TransportRail.tsx`:
```tsx
'use client';

import React, { useCallback } from 'react';
import { useRouter } from 'next/navigation';
import { cn } from '@/lib/utils';
import { useRecordingState } from '@/contexts/RecordingStateContext';
import { useSidebar } from '@/components/Sidebar/SidebarProvider';
import { recordingService } from '@/services/recordingService';
import { requestFullRecordingStop } from '@/lib/recording-stop';
import { TransportKey } from './TransportKey';
import { TransportStatus, type TransportPhase } from './TransportStatus';
import { QueueIndicator } from './QueueIndicator';

/**
 * specs/0057 decision 7 — THE recording control surface. Fixed to the bottom edge, from the
 * sidebar's right edge to the window edge, on every screen. Replaces GlobalRecordingBar,
 * the /record floating pill and the RecordingHeader button pair (one state vocabulary,
 * spec §3.1 table), and hosts the one global queue (decision 8).
 */
export function TransportRail() {
  const rs = useRecordingState();
  const { isCollapsed, handleRecordingToggle } = useSidebar();
  const router = useRouter();

  const finalizing = rs.isStopping || rs.isProcessing || rs.isSaving;
  const phase: TransportPhase = finalizing ? 'finalizing' : rs.isRecording ? (rs.isPaused ? 'paused' : 'recording') : 'idle';
  const inFlight = phase !== 'idle';
  const elapsed = rs.activeDuration ?? rs.recordingDuration ?? 0;

  const onRec = useCallback(() => {
    if (phase === 'idle') handleRecordingToggle();
  }, [phase, handleRecordingToggle]);
  const onHold = useCallback(() => {
    if (phase === 'recording') void recordingService.pauseRecording();
    else if (phase === 'paused') void recordingService.resumeRecording();
  }, [phase]);
  const onStop = useCallback(() => {
    if (phase === 'recording' || phase === 'paused') requestFullRecordingStop((p) => router.push(p));
  }, [phase, router]);

  return (
    <div
      role="region"
      aria-label={inFlight ? 'Recording in progress' : 'Transport'}
      className={cn(
        'fixed bottom-0 right-0 z-40 flex h-[var(--rail-h)] items-stretch border-t border-border',
        'bg-panel shadow-[inset_0_1px_0_hsl(var(--bevel-hi)),0_-8px_16px_-12px_rgba(0,0,0,0.35)]',
        'transition-[left] duration-300',
        isCollapsed ? 'left-16' : 'left-64',
      )}
    >
      <TransportStatus phase={phase} elapsedSeconds={elapsed} />
      <div className="w-px self-stretch bg-border" />
      <div className="flex items-center gap-1.5 px-5">
        <TransportKey fn="rec" legend="REC" lit={inFlight && phase !== 'finalizing'} dim={phase === 'paused'} disabled={phase === 'finalizing'} aria-label={phase === 'idle' ? 'Start recording' : 'Recording'} onClick={onRec} />
        <TransportKey fn="hold" legend="HOLD" lit={phase === 'paused'} disabled={phase === 'idle' || phase === 'finalizing'} aria-label={phase === 'paused' ? 'Resume recording' : 'Pause recording'} onClick={onHold} />
        <TransportKey fn="stop" legend="STOP" disabled={phase === 'idle' || phase === 'finalizing'} aria-label="Stop recording" onClick={onStop} />
      </div>
      <div className="w-px self-stretch bg-border" />
      <div className="flex-1" />
      <div className="w-px self-stretch bg-border" />
      <QueueIndicator />
    </div>
  );
}
```
`components/Transport/TransportStatus.tsx`:
```tsx
'use client';

import React from 'react';
import { useSidebar } from '@/components/Sidebar/SidebarProvider';
import { useRecordingLevel } from '@/hooks/useRecordingLevel';
import { Reels } from './Reels';
import { TapeCounter } from './TapeCounter';
import { LevelLadder } from './LevelLadder';

export type TransportPhase = 'idle' | 'recording' | 'paused' | 'finalizing';

/** Left zone of the rail: reels · title/state · counter · ladder (spec §3.1 state table). */
export function TransportStatus({ phase, elapsedSeconds }: { phase: TransportPhase; elapsedSeconds: number }) {
  const { currentMeeting, activeRecordingMeetingId } = useSidebar();
  const level = useRecordingLevel(phase === 'recording');
  const title = activeRecordingMeetingId && currentMeeting?.id === activeRecordingMeetingId ? currentMeeting.title : 'Recording';
  const line1 = phase === 'idle' ? 'Deck ready' : phase === 'finalizing' ? 'Saving' : title;
  const line2 = phase === 'idle' ? 'Nothing on the reel' : phase === 'paused' ? 'On hold' : phase === 'finalizing' ? 'Finishing the reel' : 'On the reel';
  const tone = phase === 'paused' ? 'amber' : phase === 'idle' ? 'dim' : 'normal';
  return (
    <div className="flex min-w-[380px] items-center gap-3.5 px-5">
      <Reels state={phase} />
      <div className="flex min-w-0 flex-col leading-tight">
        <span className="truncate text-xs font-semibold text-foreground">{line1}</span>
        <span className="u-section-label text-[9px]">{line2}</span>
      </div>
      <TapeCounter seconds={elapsedSeconds} size="sm" tone={tone} />
      <LevelLadder level={level.rms} active={phase === 'recording'} />
    </div>
  );
}
```
`components/Transport/QueueIndicator.tsx` — a Radix `Popover` trigger button (`aria-label="Queue"` plus visible engraved "Queue", the count, one-line status from `buildQueueView` — `running?.title` with its `stageLabel`, else `nextUp`, else "Idle" — and a `LampDot tone={view.lamp}`), rendering `<QueuePanel view={view} />` in `PopoverContent side="top" align="end"` with `role="dialog" aria-label="Queue"`. Build the view with `buildQueueView(useBacklog().view, useLlmActivity()?.view ?? null)` — wrap `useLlmActivity()` so it returns `null` when the provider isn't mounted (check whether the hook throws outside its provider; if it does, add an `useOptionalLlmActivity()` export to `LlmActivityProvider.tsx` that reads the context without throwing).
`components/Transport/QueuePanel.tsx` — `w-80` panel: header row engraved `ITEM · STAGE`, rows from `view.rows` (title, `stageLabel` engraved, `LampDot` amber for active stages/`llm`, red for `error`, green for `done`; a 6px ladder bar `bg-brand` animated with `animate-pulse`-free shimmer is NOT allowed — use a static 60% fill for active rows), then actions: `Stop` (backlog `stop`, when `processing`), `Process all` (`startNow`, when `pendingCount>0 && !processing`), `Clear finished` (`dismissDone`, when any done), and per LLM failure row `Retry`/`Dismiss` (calling the same commands `LlmActivityProvider` exposes). Copy the exact action semantics from `BacklogDetailPopover.tsx:28-57` and `LlmActivityPopover.tsx`, then leave those two files in place (ProcessMeetingsButton still opens `BacklogDetailPopover`).

- [ ] **Step 4: Mount and delete**

`app/layout.tsx`:
- import `TransportRail`; remove the `GlobalRecordingBar` and `DeferredBacklogIndicator` imports and their mounts (`:325`, `:328`).
- Change the `LlmActivityProvider` wrap so it contains BOTH the sidebar and the rail but not `MainContent`: `<LlmActivityProvider><Sidebar /><TransportRail /></LlmActivityProvider>` — keep the existing comment, amend it to say the rail's queue is the second consumer.
- `Sidebar/index.tsx`: remove `<LlmActivityRow />` (`:197`) and its import (`:4`).
- `git rm frontend/src/components/GlobalRecordingBar.tsx frontend/src/components/DeferredBacklog/DeferredBacklogIndicator.tsx frontend/src/components/LlmActivity/LlmActivityRow.tsx frontend/src/components/LlmActivity/__tests__/LlmActivityRow.test.tsx`.
- `grep -rn "GlobalRecordingBar\|DeferredBacklogIndicator\|LlmActivityRow" frontend/src` → only comments may remain; update any comment that says "DeferredBacklogIndicator owns that region".

- [ ] **Step 5: Verify**

Run: `cd frontend && pnpm test 2>&1 | tail -4 && pnpm lint && npx tsc --noEmit && cd .. && scripts/check-off-token-colors.sh && scripts/check-file-size.sh`. Launch `./dev-nixon.sh`: the rail is visible on every route; start a recording from REC on Today → it navigates to /record and starts; HOLD/STOP work from the Meetings list while recording.

- [ ] **Step 6: Commit** — `git add -A frontend/src && git commit -m "feat(0057): persistent transport rail with keys, status, and the global queue; retire GlobalRecordingBar, backlog pill, sidebar LLM row"`

---

### Task 7: Record screen — control-panel header with VU, no pill, no spectrometer

**Files:**
- Create: `frontend/src/components/Record/RecordingDeviceAlert.tsx`, `frontend/src/lib/recording-start-errors.ts`, `frontend/src/lib/__tests__/recording-start-errors.test.ts`
- Modify: `frontend/src/components/Record/RecordingHeader.tsx`, `frontend/src/app/record/page.tsx`, `frontend/src/hooks/useRecordingStart.ts`, `frontend/src/components/DeviceSelection.tsx`
- Delete: `frontend/src/components/RecordingControls.tsx`, `frontend/src/components/AudioLevelMeter.tsx`, `frontend/src/hooks/useRecordingWaveform.ts`, `frontend/src/hooks/__tests__/useRecordingWaveform.test.ts`

**Interfaces:**
- `describeRecordingStartError(err: unknown): { title: string; message: string }` — the exact title/message mapping currently in `RecordingControls.tsx:120-146` (read it; the four branches map device-not-found / permission / model-not-ready / generic).
- `<RecordingDeviceAlert error={{title,message}|null} onDismiss />` — the `Alert` block from `RecordingControls.tsx:315-335` with `aria-label="Close alert"` preserved.
- `RecordingHeader` props lose `showHeaderControls` and `recordingControlsProps`; gain nothing (VU/lamps read hooks internally).

- [ ] **Step 1: Write the failing test** for the error mapper: read the four branches at `RecordingControls.tsx:120-146` and write one `it` per branch asserting the exact `title` string the current code sets, plus a generic fallback.

- [ ] **Step 2: Run — expect FAIL**

- [ ] **Step 3: Extract**

Move the mapping into `lib/recording-start-errors.ts` (pure), and the Alert JSX into `RecordingDeviceAlert.tsx`. In `hooks/useRecordingStart.ts`, find the `catch` around the actual start call inside `handleRecordingStart` (and the `start-recording-from-sidebar` listener path if it has its own catch): call `describeRecordingStartError(err)` and surface it via the `showModal('errorAlert', …)` the hook already receives, using `${title}: ${message}` — OR, if `record/page.tsx` renders an alert slot, expose `startError` from the hook and render `<RecordingDeviceAlert>` under the header. Pick the first (smaller) unless `showModal` cannot show a title; report which.

- [ ] **Step 4: Header and page surgery**

`RecordingHeader.tsx`:
- Delete the status chip block (`:241-256`), `HeaderWaveform` (`:46-78`) and its use (`:304`), the elapsed timer (`:305-307`), the controls (`:308-310`), and the private `formatElapsed` (`:31-39`).
- Props: remove `isPaused`, `elapsedSeconds`, `showHeaderControls`, `recordingControlsProps`, and the `RecordingControlsPassthrough` type.
- Right cluster becomes: template dropdown, `ModeChip`, `ParticipantsPopover`, then a `VuMeter` block: `const level = useRecordingLevel(isRecordingActive); const gated = useMicGate();` → `<VuMeter db={rmsToVu(level.rms)} active={isRecordingActive} label="Level" />` and a lamp column `<LampDot tone={level.peakLatched ? 'red' : 'off'} label="Peak" />`, `<LampDot tone={gated ? 'amber' : 'off'} label="Mic gate" />` with engraved captions.
- Add to the identity line under the title: `Recording locally on your Mac` stays as the idle subtitle; while recording show the engraved line `On the reel` (the rail carries the counter).
- The `<header>` gets `bg-panel` and the brushed bevel (`shadow-[inset_0_1px_0_hsl(var(--bevel-hi)),inset_0_-1px_0_hsl(var(--bevel-lo))]`).

`app/record/page.tsx`:
- Remove the `motion.div` wrapper (`:135-139`) → plain `div` with the same className; remove the `framer-motion` import if unused elsewhere in the file.
- Remove the floating controls block (`:186-201`), `showFloatingControls` (`:110-114`), `recordingControlsProps` (`:118-132`), `barHeights` (`:94-98`) and the `useRecordingWaveform` import; remove `showHeaderControls`.
- Keep `useGlobalBarStop` (`:79`) — the rail's STOP still goes through `requestFullRecordingStop` → `stop-recording-from-global-bar`.

`DeviceSelection.tsx`: replace `CompactAudioLevelMeter`/`AudioLevelMeter` (`:270`, `:278`) with `<LevelLadder level={rms} active={isActive} />` (it already receives `rms`/`peak` from the `audio-levels` event); then `git rm` `AudioLevelMeter.tsx`, `RecordingControls.tsx`, `useRecordingWaveform.ts` and its test. `grep -rn "RecordingControls\|AudioLevelMeter\|useRecordingWaveform\|recording-spectrum" frontend/src` → nothing (Rust still emits the spectrum; leave it).

- [ ] **Step 5: Verify**

Run: `cd frontend && pnpm test 2>&1 | tail -4 && pnpm lint && npx tsc --noEmit && cd .. && scripts/check-off-token-colors.sh && scripts/check-file-size.sh`. Launch: on /record the header shows the VU needle moving with your voice; PEAK latches red on a clap; the rail counter runs; HOLD stops the reels dead; STOP finalizes and the summary flow proceeds as before.

- [ ] **Step 6: Commit** — `git add -A frontend/src && git commit -m "feat(0057): record header becomes the control panel (VU + lamps); retire RecordingControls, AudioLevelMeter, spectrometer"`

---

### Task 8: Channel strip with share-of-talk

**Files:**
- Create: `frontend/src/lib/speaker-talk-time.ts`, `frontend/src/lib/__tests__/speaker-talk-time.test.ts`, `frontend/src/components/MeetingDetails/ChannelStrip.tsx`, `components/MeetingDetails/__tests__/ChannelStrip.test.tsx`
- Modify: `frontend/src/components/MeetingDetails/SpeakerLegend.tsx:170-194` (container), its props (+`transcripts?: Transcript[]`), `MeetingDetails/TranscriptPanel.tsx:300-305` (pass `transcripts`)

**Interfaces:**
- `talkTimeBySpeaker(transcripts: Transcript[]): Map<string, number>` — seconds per `speaker` key using `duration ?? (audio_end_time - audio_start_time)`, ignoring rows with neither; `shareOfTalk(map): Map<string, number>` → 0..1 fractions summing to 1 (0 when total is 0).
- `<ChannelStrip rows={ChannelRow[]} renderName={(row) => ReactNode} />` where `ChannelRow = { channel: number; speakerKey: string; colorClass: string; seconds: number; share: number }`; row 1 is always the local speaker (`isLocal`), the rest in `speakers` order.

- [ ] **Step 1: Write the failing tests**

`lib/__tests__/speaker-talk-time.test.ts`:
```ts
import { describe, it, expect } from 'vitest';
import { talkTimeBySpeaker, shareOfTalk } from '@/lib/speaker-talk-time';
import type { Transcript } from '@/types';

const t = (speaker: string | null, a?: number, b?: number, duration?: number): Transcript =>
  ({ id: Math.random().toString(), text: 'x', timestamp: '', speaker, audio_start_time: a, audio_end_time: b, duration } as Transcript);

// specs/0057 §3.5 — share-of-talk is nearly free: the data is already in the transcript rows.
describe('talkTimeBySpeaker', () => {
  it('sums duration, falling back to end-start, per speaker key', () => {
    const m = talkTimeBySpeaker([t('local', 0, 10), t('spk_0', 10, 14, 4), t('local', 20, 25), t(null, 0, 99), t('spk_1')]);
    expect(m.get('local')).toBe(15);
    expect(m.get('spk_0')).toBe(4);
    expect(m.has('spk_1')).toBe(false);
    expect(m.has('')).toBe(false);
  });
  it('shares sum to one and are zero on an empty meeting', () => {
    const s = shareOfTalk(new Map([['local', 30], ['spk_0', 10]]));
    expect(s.get('local')).toBeCloseTo(0.75);
    expect(s.get('spk_0')).toBeCloseTo(0.25);
    expect(shareOfTalk(new Map()).size).toBe(0);
  });
});
```
`components/MeetingDetails/__tests__/ChannelStrip.test.tsx`:
```tsx
import { describe, it, expect } from 'vitest';
import { render, screen } from '@testing-library/react';
import { ChannelStrip } from '@/components/MeetingDetails/ChannelStrip';

describe('ChannelStrip', () => {
  it('renders one row per channel with time and share, CH1 first', () => {
    render(
      <ChannelStrip
        rows={[
          { channel: 1, speakerKey: 'local', colorClass: 'bg-chart-1', seconds: 1450, share: 0.58 },
          { channel: 2, speakerKey: 'spk_0', colorClass: 'bg-chart-2', seconds: 692, share: 0.27 },
        ]}
        renderName={(r) => <span>{r.speakerKey === 'local' ? 'You' : 'Sarah Chen'}</span>}
      />,
    );
    const rows = screen.getAllByRole('row');
    expect(rows.length).toBe(3); // header + 2
    expect(rows[1].textContent).toMatch(/1/);
    expect(rows[1].textContent).toMatch(/You/);
    expect(rows[1].textContent).toMatch(/24:10/);
    expect(rows[1].textContent).toMatch(/58%/);
    expect(rows[2].textContent).toMatch(/Sarah Chen/);
  });
});
```

- [ ] **Step 2: Run — expect FAIL**

- [ ] **Step 3: Implement**

`lib/speaker-talk-time.ts`:
```ts
import type { Transcript } from '@/types';

/** Seconds of speech per speaker key (specs/0057 §3.5). Rows without timing are ignored. */
export function talkTimeBySpeaker(transcripts: Transcript[]): Map<string, number> {
  const out = new Map<string, number>();
  for (const t of transcripts) {
    const key = t.speaker;
    if (!key) continue;
    const d = typeof t.duration === 'number' ? t.duration
      : typeof t.audio_start_time === 'number' && typeof t.audio_end_time === 'number' ? t.audio_end_time - t.audio_start_time
      : null;
    if (d === null || !(d > 0)) continue;
    out.set(key, (out.get(key) ?? 0) + d);
  }
  return out;
}

export function shareOfTalk(seconds: Map<string, number>): Map<string, number> {
  let total = 0;
  seconds.forEach((v) => (total += v));
  const out = new Map<string, number>();
  if (total <= 0) return out;
  seconds.forEach((v, k) => out.set(k, v / total));
  return out;
}

/** m:ss or h:mm:ss for the TIME column. */
export function formatTalkTime(seconds: number): string {
  const s = Math.max(0, Math.floor(seconds));
  const h = Math.floor(s / 3600), m = Math.floor((s % 3600) / 60), sec = s % 60;
  return h > 0 ? `${h}:${String(m).padStart(2, '0')}:${String(sec).padStart(2, '0')}` : `${m}:${String(sec).padStart(2, '0')}`;
}
```
`components/MeetingDetails/ChannelStrip.tsx`:
```tsx
import React from 'react';
import { cn } from '@/lib/utils';
import { formatTalkTime } from '@/lib/speaker-talk-time';

export interface ChannelRow {
  channel: number;
  speakerKey: string;
  colorClass: string; // bg-chart-n
  seconds: number;
  share: number; // 0..1
}

/**
 * specs/0057 §3.5 — a multitrack deck labels its inputs in a fixed-width column. Replaces the
 * speaker chip cloud: CH · color bar · name (the existing SpeakerChip, so rename/merge/assign
 * behave exactly as before) · time · share ladder · %.
 */
export function ChannelStrip({ rows, renderName, className }: { rows: ChannelRow[]; renderName: (row: ChannelRow) => React.ReactNode; className?: string }) {
  return (
    <div role="table" aria-label="Channels" className={cn('rounded-[3px] border border-border bg-card px-3.5 pb-2 pt-1.5', className)}>
      <div role="row" className="grid h-[22px] grid-cols-[36px_1fr_64px_minmax(120px,220px)_40px] items-center gap-x-3">
        <span role="columnheader" className="u-section-label text-[9px]">CH</span>
        <span role="columnheader" className="u-section-label text-[9px]">Speaker</span>
        <span role="columnheader" className="u-section-label text-right text-[9px]">Time</span>
        <span role="columnheader" className="u-section-label text-[9px]">Share of talk</span>
        <span role="columnheader" />
      </div>
      <div className="h-px bg-border" />
      {rows.map((r) => (
        <div key={r.speakerKey} role="row" className="grid min-h-[26px] grid-cols-[36px_1fr_64px_minmax(120px,220px)_40px] items-center gap-x-3">
          <span role="cell" className="flex items-center gap-2 text-[11px] font-semibold text-engrave">
            <i className={cn('block h-3.5 w-[3px]', r.colorClass)} aria-hidden />
            {r.channel}
          </span>
          <span role="cell" className="min-w-0 text-[13px] text-foreground">{renderName(r)}</span>
          <span role="cell" className="text-right text-xs tabular-nums text-muted-foreground">{formatTalkTime(r.seconds)}</span>
          <span role="cell" className="relative h-1.5 bg-well shadow-[inset_0_1px_1px_rgba(0,0,0,0.5)]">
            <i className={cn('absolute inset-y-0 left-0', r.colorClass)} style={{ width: `${Math.round(r.share * 100)}%` }} aria-hidden />
          </span>
          <span role="cell" className="text-right text-[11px] tabular-nums text-muted-foreground">{Math.round(r.share * 100)}%</span>
        </div>
      ))}
    </div>
  );
}
```

- [ ] **Step 4: Swap it into SpeakerLegend**

`SpeakerLegend.tsx`: add prop `transcripts?: Transcript[]` (default `[]`). Replace the flex-wrap container (`:170-194`) with:
```tsx
const seconds = useMemo(() => talkTimeBySpeaker(transcripts), [transcripts]);
const shares = useMemo(() => shareOfTalk(seconds), [seconds]);
const rows: ChannelRow[] = visibleSpeakers.map((s, i) => ({
  channel: i + 1, speakerKey: s.speakerKey,
  colorClass: speakerBgClass(s.isLocal ? 'local' : s.speakerKey),
  seconds: seconds.get(s.speakerKey) ?? 0, share: shares.get(s.speakerKey) ?? 0,
}));
<ChannelStrip rows={rows} renderName={(r) => <SpeakerChip {...propsForSpeaker(r.speakerKey)} />} />
```
where `visibleSpeakers` is the existing local/remote ordering (local first — it already is; confirm) respecting the existing `COLLAPSE_THRESHOLD`/Show-all toggle, and `propsForSpeaker` is the existing per-chip props construction from the removed loop. **`SpeakerLegend.tsx` must end at ≤689 lines** — the removed `flex-wrap` markup pays for the new lines; if it does not, move `propsForSpeaker` into a new `MeetingDetails/speaker-chip-props.ts`. `SpeakerChip` itself is unchanged (it stays as the interactive name cell).

`MeetingDetails/TranscriptPanel.tsx:300-305`: pass `transcripts={transcripts}` (the panel already holds the meeting's transcript array — find its variable name).

- [ ] **Step 5: Verify**

Run: `cd frontend && pnpm test 2>&1 | tail -4 && pnpm lint && npx tsc --noEmit && cd .. && scripts/check-file-size.sh && scripts/check-off-token-colors.sh`. Launch: open a past meeting's Transcript tab: rows CH1 You … with times and shares; rename/merge/assign still work from the name cell.

- [ ] **Step 6: Commit** — `git add -A frontend/src && git commit -m "feat(0057): channel strip with share-of-talk replaces the speaker chip cloud"`

---

### Task 9: Docs, DoD run, handover

**Files:**
- Modify: `CHANGELOG.md` (Unreleased), `docs/MANUAL_SMOKE.md`, `specs/0057-nixon-rebrand-and-tape-deck-redesign.md` (Plan 2 residuals section)

- [ ] **Step 1: CHANGELOG** — under the rebrand bullet add:
```markdown
- **Transport rail** (specs/0057 Phase C): one persistent bottom rail on every screen carries
  REC / HOLD / STOP, the reels, the tape counter, a level ladder, and the single **Queue** for
  deferred processing and background AI. The floating recording pill, the global recording bar,
  the sidebar AI-activity row and the backlog pill are gone.
- **VU meter** with real needle ballistics on the Record screen, plus PEAK and MIC GATE lamps
  (the Zoom mute gate is finally visible). The spectrometer strip is retired.
- **Channel strip**: the speaker list on a meeting shows CH numbers, talk time and share of
  talk; CH 1 is always you.
### Fixed
- New recordings are written to the folder shown in Settings (the persisted preference), not
  to a folder guessed from disk state; this closes the "Access denied … outside the app's
  allowed data directories" path after a folder rename or machine migration.
```
- [ ] **Step 2: MANUAL_SMOKE.md** — replace the recording-controls steps with: rail visible on Today/Meetings/Settings; REC from Today starts and navigates; HOLD/STOP from another screen; counter/reels/ladder states; VU needle + PEAK on /record; MIC GATE lamp while muted in Zoom; Queue count/lamp/panel actions; channel strip rows.
- [ ] **Step 3: Full DoD** (see Global Constraints) — record outputs in the report.
- [ ] **Step 4: Spec residuals** — append "Plan 2 residuals" to the spec: two-channel VU (needs per-channel rms from `pipeline.rs`), DROPOUT lamp wiring (`transcriptGaps` in `app/_components/TranscriptPanel.tsx:107-119`), channel-strip relocation above the tabs (page-content), `REEL nnnn` ordinal (no DB column; derive `ROW_NUMBER() OVER (ORDER BY created_at)` in `api_get_meetings`), tray icons by state, `DOT_CLASSES` avatar rotation, and anything the reviews parked.
- [ ] **Step 5: Commit** — `git commit -am "docs(0057): changelog + smoke steps for the transport rail, VU, channel strip; Plan 2 residuals"`. Do not push.

---

## Self-review notes

- **Spec coverage (Phase C, decisions 6–9):** rail (T6), keys + state table incl. mic-gate lamp (T3, T4, T6, T7), VU needle + ladder + spectrometer retired (T4, T7), counter (T3), reels (T4), channel strip with share (T8), one global queue + `LlmActivityRow` deleted + `ProcessMeetingsButton` kept (T5, T6), record header simplified (T7), write-root residual (T1). Deliberate deviations recorded: ONE mixed-signal VU instead of two per-channel meters (backend emits a single `recording-level`; per-channel is a Plan 3 residual) — spec §3.2 asked for two; the DROPOUT lamp is deferred; the channel strip stays in the Transcript tab this plan (relocation above the tabs is per-screen work).
- **Type consistency:** `TransportPhase` defined in `TransportStatus.tsx` and consumed in `TransportRail.tsx`; `formatElapsedHms` (T3) used by `TapeCounter`; `rmsToVu`/`vuToArcFraction`/`createVuIntegrator` (T4) used by `VuMeter` and `RecordingHeader` (T7); `ladderSegments` → `LevelLadder`; `buildQueueView` (T5) → `QueueIndicator` (T6); `ChannelRow` (T8) consumed by `SpeakerLegend`.
- **Ratchet:** every new component is a new file; `SpeakerLegend.tsx` must not grow; `RecordingHeader.tsx` shrinks; `record/page.tsx` shrinks.
- **Aria preservation:** `"Pause recording"` / `"Resume recording"` / `"Stop recording"` / `"Recording in progress"` / `"Close alert"` all kept; `"Start recording"` is new on REC (was a tooltip).
