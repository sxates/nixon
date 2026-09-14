# Deferred-Processing UX (spec 0045) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make deferred/low-power meeting processing run regardless of recording state, auto-start on AC power, and surface it through one shared, honest, top-level control (Today-header button + global indicator + per-meeting status) — fixing the silent-no-op "Process now" button.

**Architecture:** Turn the single-instance `useDeferredBacklog` hook into a **context provider** so three UI surfaces read one shared backlog state. Move all queue state into a pure, per-meeting-keyed reducer + selectors in `lib/deferred-backlog.ts`, and the auto-start decision into a new pure `lib/backlog-autostart.ts`, keeping the hook a thin async driver. Remove every `isRecording` gate. Backend: serialize STT decode so concurrent live + batch transcription can't race on the shared engine context.

**Tech Stack:** Next.js 14 / React 18 / TypeScript, Tauri 2 IPC (`invoke`/`listen`), Vitest (jsdom) for frontend tests; Rust (Tauri commands, `tokio`, `whisper-rs`/Parakeet) with `cargo test --features metal`.

## Global Constraints

- **File-size ratchet (spec 0042):** no production source file over **800 lines** unless allowlisted; allowlisted files may only shrink. New trigger/queue logic goes in `lib/backlog-autostart.ts` and stays in `lib/deferred-backlog.ts`, NOT by growing `useDeferredBacklog.ts`. Run `scripts/check-file-size.sh`.
- **Definition of Done (`/CLAUDE.md`):** `cargo check`, `cargo clippy`, `cargo test` clean in `frontend/src-tauri`; `pnpm lint` and `pnpm test` clean in `frontend`; app still launches via `./clean_run.sh`; record→transcript→summary smoke still works.
- **Known-failing test stays exempt:** the `vad_filter` adversarial-fixtures test on macOS 15.x is a pre-existing failure — do not treat it as a regression.
- **Rust conventions:** `anyhow::Result`; Tauri command (FE→Rust) + event (Rust→FE) pattern; audio devices named "microphone"/"system".
- **Frontend conventions:** Tauri `invoke`/`listen`; user-friendly try/catch messages; never hardcode paths.
- **Commit cadence:** commit after each task's tests pass. Branch is `0045-1.11-feedback-batch` (already created off `main`).
- **`cargo fmt --check` is NOT clean repo-wide** (pre-existing drift) — format only files you touch.

---

## File Structure

**Create:**
- `frontend/src/lib/backlog-autostart.ts` — pure auto-start decision + debounce constant.
- `frontend/src/lib/__tests__/backlog-autostart.test.ts` — its unit tests.
- `frontend/src/contexts/DeferredBacklogProvider.tsx` — context that instantiates the controller once; `useBacklog()` consumer.
- `frontend/src/components/DeferredBacklog/ProcessMeetingsButton.tsx` — Today-header button (reads context).
- `frontend/src/components/DeferredBacklog/DeferredBacklogIndicator.tsx` — global compact indicator (replaces the nag pill).
- `frontend/src/components/DeferredBacklog/BacklogDetailPopover.tsx` — per-meeting status list.

**Modify:**
- `frontend/src/lib/deferred-backlog.ts` — new per-meeting-keyed state model, reducer, selectors; drop prompt/debounce logic.
- `frontend/src/lib/__tests__/deferred-backlog.test.ts` — rewrite tests for the new model.
- `frontend/src/hooks/useDeferredBacklog.ts` — remove `isRecording` gating; drain loop; enqueue/stop; auto-start; expose view model.
- `frontend/src/app/layout.tsx` — wrap app in `DeferredBacklogProvider`; swap `<DeferredBacklogPrompt/>` → `<DeferredBacklogIndicator/>`.
- `frontend/src/components/Today/TodayHeader.tsx` — render `<ProcessMeetingsButton/>`.
- `frontend/src/components/MeetingDetails/TranscriptButtonGroup.tsx` — delete local `processRequested`; read status from context; enqueue on click.
- `frontend/src/components/MeetingDetails/TranscriptPanel.tsx` — compute `unprocessed` and pass to the transcript view.
- `frontend/src/components/VirtualizedTranscriptView.tsx` — new `unprocessed`/`onProcessNow` props; context-aware empty state.
- `frontend/src-tauri/src/whisper_engine/whisper_engine.rs` + `frontend/src-tauri/src/parakeet_engine/parakeet_engine.rs` — serialize decode via a shared inference lock.
- `frontend/src-tauri/src/audio/mod.rs` (or a small new `audio/stt_lock.rs`) — the inference lock.

**Delete:**
- `frontend/src/components/DeferredBacklog/DeferredBacklogPrompt.tsx` (replaced by the indicator).

---

## Task 1: Pure auto-start decision (`backlog-autostart.ts`)

**Files:**
- Create: `frontend/src/lib/backlog-autostart.ts`
- Test: `frontend/src/lib/__tests__/backlog-autostart.test.ts`

**Interfaces:**
- Produces: `decideAutostart(input: { backlogCount: number; onBattery: boolean; isProcessing: boolean }): boolean`; `AUTOSTART_DEBOUNCE_MS: number` (5000).

- [ ] **Step 1: Write the failing test**

```ts
// frontend/src/lib/__tests__/backlog-autostart.test.ts
import { describe, it, expect } from 'vitest';
import { decideAutostart, AUTOSTART_DEBOUNCE_MS } from '@/lib/backlog-autostart';

describe('decideAutostart', () => {
  it('auto-starts when a backlog exists, on AC, and not already processing', () => {
    expect(decideAutostart({ backlogCount: 2, onBattery: false, isProcessing: false })).toBe(true);
  });
  it('never auto-starts on battery (defeats the battery-saving purpose)', () => {
    expect(decideAutostart({ backlogCount: 2, onBattery: true, isProcessing: false })).toBe(false);
  });
  it('does not auto-start with an empty backlog', () => {
    expect(decideAutostart({ backlogCount: 0, onBattery: false, isProcessing: false })).toBe(false);
  });
  it('does not start a second queue while one is running', () => {
    expect(decideAutostart({ backlogCount: 3, onBattery: false, isProcessing: true })).toBe(false);
  });
  it('uses a short blip-absorbing debounce', () => {
    expect(AUTOSTART_DEBOUNCE_MS).toBe(5000);
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd frontend && pnpm test -- backlog-autostart`
Expected: FAIL — cannot resolve `@/lib/backlog-autostart`.

- [ ] **Step 3: Write minimal implementation**

```ts
// frontend/src/lib/backlog-autostart.ts
/**
 * Deferred-backlog auto-start decision (spec 0045 WS2).
 *
 * When a backlog of unprocessed (low-power/deferred) meetings exists AND the machine is on
 * AC power, processing auto-starts — no prompt. On battery it never auto-starts (that would
 * defeat the battery-saving purpose); the user can still start it manually. This is the pure,
 * unit-testable decision behind the trigger in useDeferredBacklog.
 */

/** Short debounce after a triggering signal (power change / mount / enqueue) before we act,
 *  to absorb brief power blips. Replaces the old 90s/120s prompt debounces. */
export const AUTOSTART_DEBOUNCE_MS = 5000;

export function decideAutostart(input: {
  backlogCount: number;
  onBattery: boolean;
  isProcessing: boolean;
}): boolean {
  return input.backlogCount > 0 && !input.onBattery && !input.isProcessing;
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd frontend && pnpm test -- backlog-autostart`
Expected: PASS (5 tests).

- [ ] **Step 5: Commit**

```bash
git add frontend/src/lib/backlog-autostart.ts frontend/src/lib/__tests__/backlog-autostart.test.ts
git commit -m "feat(0045): pure decideAutostart + AUTOSTART_DEBOUNCE_MS (WS2)"
```

---

## Task 2: Per-meeting backlog state model + selectors (`deferred-backlog.ts`)

Replace the prompt/queue reducer with a per-meeting-keyed status model that all three UI surfaces render from. Keep `DeferredMeeting` and `needsRetranscription`.

**Files:**
- Modify: `frontend/src/lib/deferred-backlog.ts`
- Test: `frontend/src/lib/__tests__/deferred-backlog.test.ts` (rewrite the reducer/label sections; keep the `needsRetranscription` tests)

**Interfaces:**
- Consumes: `DeferredMeeting` (unchanged: `{ id; title; folderPath; transcriptCount }`).
- Produces:
  - `type BacklogItemStatus = 'waiting' | 'transcribing' | 'diarizing' | 'summarizing' | 'done' | 'error'`
  - `interface BacklogItem { meeting: DeferredMeeting; status: BacklogItemStatus }`
  - `interface BacklogState { items: BacklogItem[]; processing: boolean }`
  - `initialBacklogState: BacklogState`
  - `backlogReducer(state, action)` with actions: `{type:'enqueue'; meetings: DeferredMeeting[]}`, `{type:'set-status'; meetingId: string; status: BacklogItemStatus}`, `{type:'processing-started'}`, `{type:'processing-stopped'}`, `{type:'clear-done'}`, `{type:'reset'}`.
  - `interface BacklogView { items: BacklogItem[]; pendingCount: number; processing: boolean; active: BacklogItem | null; activeOrdinal: number; total: number }`
  - `selectBacklogView(state: BacklogState): BacklogView`
  - `backlogItemStatus(state: BacklogState, meetingId: string): BacklogItemStatus | null`
  - `firstWaiting(state: BacklogState): BacklogItem | null`
  - `needsRetranscription(transcriptCount, explicitlyDeferred)` (unchanged).

- [ ] **Step 1: Write the failing test** (replace the reducer/`backlogProgressLabel`/`shouldPromptBacklog` describe blocks; KEEP the existing `needsRetranscription` block)

```ts
// frontend/src/lib/__tests__/deferred-backlog.test.ts  (new reducer/selectors sections)
import { describe, it, expect } from 'vitest';
import {
  backlogReducer,
  initialBacklogState,
  selectBacklogView,
  backlogItemStatus,
  firstWaiting,
  type BacklogState,
  type DeferredMeeting,
} from '@/lib/deferred-backlog';

const m = (id: string, title = id): DeferredMeeting => ({
  id, title, folderPath: `/rec/${id}`, transcriptCount: 0,
});

describe('backlogReducer', () => {
  it('enqueues meetings as waiting, de-duped by id', () => {
    let s = backlogReducer(initialBacklogState, { type: 'enqueue', meetings: [m('a'), m('b')] });
    s = backlogReducer(s, { type: 'enqueue', meetings: [m('b'), m('c')] });
    expect(s.items.map((i) => i.meeting.id)).toEqual(['a', 'b', 'c']);
    expect(s.items.every((i) => i.status === 'waiting')).toBe(true);
  });

  it('re-enqueuing a done/error meeting resets it to waiting', () => {
    let s = backlogReducer(initialBacklogState, { type: 'enqueue', meetings: [m('a')] });
    s = backlogReducer(s, { type: 'set-status', meetingId: 'a', status: 'done' });
    s = backlogReducer(s, { type: 'enqueue', meetings: [m('a')] });
    expect(backlogItemStatus(s, 'a')).toBe('waiting');
  });

  it('set-status updates one item; processing flags toggle', () => {
    let s = backlogReducer(initialBacklogState, { type: 'enqueue', meetings: [m('a'), m('b')] });
    s = backlogReducer(s, { type: 'processing-started' });
    s = backlogReducer(s, { type: 'set-status', meetingId: 'a', status: 'transcribing' });
    expect(s.processing).toBe(true);
    expect(backlogItemStatus(s, 'a')).toBe('transcribing');
    expect(backlogItemStatus(s, 'b')).toBe('waiting');
    s = backlogReducer(s, { type: 'processing-stopped' });
    expect(s.processing).toBe(false);
  });

  it('clear-done drops only completed items', () => {
    let s = backlogReducer(initialBacklogState, { type: 'enqueue', meetings: [m('a'), m('b')] });
    s = backlogReducer(s, { type: 'set-status', meetingId: 'a', status: 'done' });
    s = backlogReducer(s, { type: 'clear-done' });
    expect(s.items.map((i) => i.meeting.id)).toEqual(['b']);
  });
});

describe('selectBacklogView', () => {
  it('reports pending count, active item, and k-of-N ordinals', () => {
    let s: BacklogState = backlogReducer(initialBacklogState, {
      type: 'enqueue', meetings: [m('a', 'Standup'), m('b', 'Review'), m('c', 'Sync')],
    });
    s = backlogReducer(s, { type: 'processing-started' });
    s = backlogReducer(s, { type: 'set-status', meetingId: 'a', status: 'done' });
    s = backlogReducer(s, { type: 'set-status', meetingId: 'b', status: 'diarizing' });
    const v = selectBacklogView(s);
    expect(v.processing).toBe(true);
    expect(v.pendingCount).toBe(2); // b (active) + c (waiting); done 'a' excluded
    expect(v.active?.meeting.id).toBe('b');
    expect(v.activeOrdinal).toBe(2);
    expect(v.total).toBe(3);
  });

  it('no active item when idle', () => {
    const v = selectBacklogView(initialBacklogState);
    expect(v.active).toBeNull();
    expect(v.pendingCount).toBe(0);
    expect(v.processing).toBe(false);
  });
});

describe('firstWaiting', () => {
  it('returns the earliest waiting item, or null when none', () => {
    let s = backlogReducer(initialBacklogState, { type: 'enqueue', meetings: [m('a'), m('b')] });
    s = backlogReducer(s, { type: 'set-status', meetingId: 'a', status: 'done' });
    expect(firstWaiting(s)?.meeting.id).toBe('b');
    s = backlogReducer(s, { type: 'set-status', meetingId: 'b', status: 'done' });
    expect(firstWaiting(s)).toBeNull();
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd frontend && pnpm test -- deferred-backlog`
Expected: FAIL — `selectBacklogView`, `backlogItemStatus`, `firstWaiting`, new action shapes don't exist.

- [ ] **Step 3: Write minimal implementation** — replace lines 22–147 of `deferred-backlog.ts` (the `ProcessStep`/`BacklogPhase`/`BacklogState`/reducer/`shouldPromptBacklog`/`backlogProgressLabel`/`ON_MOUNT_GRACE_MS`/`AC_TRANSITION_DEBOUNCE_MS` block) with the following. KEEP the top-of-file `DeferredMeeting` interface (lines 22–28) and the `needsRetranscription` function (lines 83–93) — move `DeferredMeeting` above the new model if needed. Delete the now-unused `SPARSE_TRANSCRIPT_SEGMENTS` import ONLY if `needsRetranscription` no longer references it (it still does — keep the import).

```ts
/** The per-meeting pipeline status shown in the UI. */
export type BacklogItemStatus =
  | 'waiting'
  | 'transcribing'
  | 'diarizing'
  | 'summarizing'
  | 'done'
  | 'error';

export interface BacklogItem {
  meeting: DeferredMeeting;
  status: BacklogItemStatus;
}

/**
 * Backlog state: an ordered list of items (done items are retained for the session so the
 * user can see what finished) plus whether a drain is currently running. Keyed by meeting id
 * so every surface — Today button, global indicator, per-meeting button — reads one truth.
 */
export interface BacklogState {
  items: BacklogItem[];
  processing: boolean;
}

export const initialBacklogState: BacklogState = { items: [], processing: false };

export type BacklogAction =
  | { type: 'enqueue'; meetings: DeferredMeeting[] }
  | { type: 'set-status'; meetingId: string; status: BacklogItemStatus }
  | { type: 'processing-started' }
  | { type: 'processing-stopped' }
  | { type: 'clear-done' }
  | { type: 'reset' };

const TERMINAL: BacklogItemStatus[] = ['done', 'error'];

export function backlogReducer(state: BacklogState, action: BacklogAction): BacklogState {
  switch (action.type) {
    case 'enqueue': {
      const items = [...state.items];
      for (const meeting of action.meetings) {
        const existing = items.findIndex((i) => i.meeting.id === meeting.id);
        if (existing === -1) {
          items.push({ meeting, status: 'waiting' });
        } else if (TERMINAL.includes(items[existing].status)) {
          // A previously finished/failed meeting was asked for again — requeue it.
          items[existing] = { meeting, status: 'waiting' };
        }
        // else: already waiting/active — leave it (no duplicate, no silent drop).
      }
      return { ...state, items };
    }
    case 'set-status':
      return {
        ...state,
        items: state.items.map((i) =>
          i.meeting.id === action.meetingId ? { ...i, status: action.status } : i,
        ),
      };
    case 'processing-started':
      return { ...state, processing: true };
    case 'processing-stopped':
      return { ...state, processing: false };
    case 'clear-done':
      return { ...state, items: state.items.filter((i) => !TERMINAL.includes(i.status)) };
    case 'reset':
      return initialBacklogState;
    default:
      return state;
  }
}

/** The earliest still-waiting item, or null. Drives the drain loop. */
export function firstWaiting(state: BacklogState): BacklogItem | null {
  return state.items.find((i) => i.status === 'waiting') ?? null;
}

/** Current status of one meeting, or null if it isn't in the backlog. */
export function backlogItemStatus(
  state: BacklogState,
  meetingId: string,
): BacklogItemStatus | null {
  return state.items.find((i) => i.meeting.id === meetingId)?.status ?? null;
}

/** Aggregated view for the UI surfaces. */
export interface BacklogView {
  items: BacklogItem[];
  /** Items not yet in a terminal state (waiting/active). */
  pendingCount: number;
  processing: boolean;
  /** The item currently being processed (first non-terminal, non-waiting), else null. */
  active: BacklogItem | null;
  /** 1-based position of the active item among all items, for "k of N". */
  activeOrdinal: number;
  /** Total items in this session's queue (incl. done), for "k of N". */
  total: number;
}

const ACTIVE_STATES: BacklogItemStatus[] = ['transcribing', 'diarizing', 'summarizing'];

export function selectBacklogView(state: BacklogState): BacklogView {
  const pendingCount = state.items.filter((i) => !TERMINAL.includes(i.status)).length;
  const activeIndex = state.items.findIndex((i) => ACTIVE_STATES.includes(i.status));
  const active = activeIndex === -1 ? null : state.items[activeIndex];
  return {
    items: state.items,
    pendingCount,
    processing: state.processing,
    active,
    activeOrdinal: activeIndex === -1 ? 0 : activeIndex + 1,
    total: state.items.length,
  };
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd frontend && pnpm test -- deferred-backlog`
Expected: PASS (reducer + selector + firstWaiting + retained `needsRetranscription` tests).

- [ ] **Step 5: Commit**

```bash
git add frontend/src/lib/deferred-backlog.ts frontend/src/lib/__tests__/deferred-backlog.test.ts
git commit -m "feat(0045): per-meeting backlog state model + selectors (WS3 core)"
```

---

## Task 3: Rewire the controller — remove recording gating, drain loop, auto-start, expose view

Rewrite `useDeferredBacklog` to drive the new model: no `isRecording` gating anywhere, a self-refilling drain loop (so a click while busy just enqueues and is picked up — no silent drop), auto-start on AC, and a returned view model + `enqueueMeeting`/`stop`/`dismissDone`.

**Files:**
- Modify: `frontend/src/hooks/useDeferredBacklog.ts`

**Interfaces:**
- Consumes: `decideAutostart`, `AUTOSTART_DEBOUNCE_MS` (Task 1); `BacklogView`, `selectBacklogView`, `backlogReducer`, `initialBacklogState`, `firstWaiting`, `needsRetranscription`, `DeferredMeeting` (Task 2).
- Produces: `useDeferredBacklog(): UseDeferredBacklogReturn` where
  ```ts
  export interface UseDeferredBacklogReturn {
    view: BacklogView;
    /** Add a meeting to the backlog and (on AC, or forced) start draining. */
    enqueueMeeting: (meetingId: string, opts?: { force?: boolean }) => void;
    /** Signal the drain loop to stop after the current step. */
    stop: () => void;
    /** Drop completed/errored items from the list. */
    dismissDone: () => void;
    /** Start draining now (manual button), regardless of power. */
    startNow: () => void;
  }
  ```

- [ ] **Step 1: Update imports and the return type** — replace the import block (lines 38–47) and `UseDeferredBacklogReturn` (lines 69–88):

```ts
import {
  backlogReducer,
  initialBacklogState,
  selectBacklogView,
  firstWaiting,
  needsRetranscription,
  type BacklogView,
  type DeferredMeeting,
} from '@/lib/deferred-backlog';
import { decideAutostart, AUTOSTART_DEBOUNCE_MS } from '@/lib/backlog-autostart';
```

```ts
export interface UseDeferredBacklogReturn {
  view: BacklogView;
  enqueueMeeting: (meetingId: string, opts?: { force?: boolean }) => void;
  stop: () => void;
  dismissDone: () => void;
  startNow: () => void;
}
```

- [ ] **Step 2: Replace refs, remove recording state** — in the hook body (lines 172–200), remove `useRecordingState` usage and the `isRecordingRef`, `phaseRef`, `queueRef` refs and their sync effects; add a `stopRef` and an `itemsRef` mirror. Keep `processingRef`, `transcriptProviderRef`, summary polling refs, `checkTimerRef`.

```ts
  const [state, dispatch] = useReducer(backlogReducer, initialBacklogState);
  const { startSummaryPolling, stopSummaryPolling } = useSidebar();
  const { transcriptModelConfig } = useConfig();

  const transcriptProviderRef = useRef(transcriptModelConfig?.provider);
  const startSummaryPollingRef = useRef(startSummaryPolling);
  const stopSummaryPollingRef = useRef(stopSummaryPolling);
  const processingRef = useRef(false);
  const stopRef = useRef(false);
  const itemsRef = useRef(state.items);
  const checkTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    itemsRef.current = state.items;
  }, [state.items]);
  useEffect(() => {
    transcriptProviderRef.current = transcriptModelConfig?.provider;
  }, [transcriptModelConfig?.provider]);
  useEffect(() => {
    startSummaryPollingRef.current = startSummaryPolling;
    stopSummaryPollingRef.current = stopSummaryPolling;
  }, [startSummaryPolling, stopSummaryPolling]);
```

- [ ] **Step 3: Make the wait un-abortable-by-recording** — in `awaitMeetingCompletion` calls, replace `shouldAbort: () => isRecordingRef.current` with `shouldAbort: () => stopRef.current` in BOTH `runRetranscription` (line 224) and `runDiarization` (line 244), and in `runSummary`'s guard (line 321 area — change the same predicate). The wait now aborts only on an explicit user `stop`, never on recording.

```ts
// runRetranscription (was line 224)
{ timeoutMs: RETRANSCRIPTION_TIMEOUT_MS, shouldAbort: () => stopRef.current },
// runDiarization (was line 244)
{ timeoutMs: DIARIZATION_TIMEOUT_MS, shouldAbort: () => stopRef.current },
```

- [ ] **Step 4: Rewrite `processMeeting` keyed by meeting id (no recording checks)** — replace the whole `processMeeting` (lines 345–397). It now takes a single meeting, dispatches per-step `set-status`, and returns nothing meaningful (status lives in state):

```ts
  const processMeeting = useCallback(async (m: DeferredMeeting): Promise<void> => {
    const explicitlyDeferred = true; // backlog meetings are all defer-marked or sparse
    // 1. Retranscribe when sparse or explicitly deferred.
    if (needsRetranscription(m.transcriptCount, explicitlyDeferred)) {
      dispatch({ type: 'set-status', meetingId: m.id, status: 'transcribing' });
      const status = await runRetranscription(m);
      if (status === 'aborted') return; // user stop; leave it waiting-cleared below
      if (status !== 'complete') {
        dispatch({ type: 'set-status', meetingId: m.id, status: 'error' });
        return; // marker stays on the backend; retried on a later pass
      }
    }
    if (stopRef.current) return;

    // 2. Diarize (best-effort — a timeout/error just skips it).
    dispatch({ type: 'set-status', meetingId: m.id, status: 'diarizing' });
    if ((await runDiarization(m)) === 'aborted') return;
    if (stopRef.current) return;

    // 3. Summarize.
    dispatch({ type: 'set-status', meetingId: m.id, status: 'summarizing' });
    const summary = await runSummary(m);
    if (summary === 'aborted') return;

    // 4. Clear the pending marker.
    try {
      await invoke('api_set_meeting_processing_mode', { meetingId: m.id, mode: null });
    } catch (error) {
      console.warn('[deferred-backlog] failed to clear processing mode:', error);
    }
    dispatch({
      type: 'set-status',
      meetingId: m.id,
      status: summary === 'error' ? 'error' : 'done',
    });
  }, [runRetranscription, runDiarization, runSummary]);
```

- [ ] **Step 5: Replace `runQueue` with a self-refilling `drain`** — replace `runQueue` (lines 399–465). It pulls the next waiting item from `itemsRef` each iteration, so meetings enqueued mid-run are picked up. It removes the `isRecording` abort, the accept/prompt bookkeeping, and the completion toast (replaced by persistent Done state); it keeps a single non-repeating summary toast only for failures.

```ts
  const drain = useCallback(async (): Promise<void> => {
    if (processingRef.current) return; // a drain is already running; it will pick up new items
    processingRef.current = true;
    stopRef.current = false;
    dispatch({ type: 'processing-started' });
    const failedTitles: string[] = [];
    try {
      for (;;) {
        if (stopRef.current) break;
        const next = firstWaiting({ items: itemsRef.current, processing: true });
        if (!next) break;
        await processMeeting(next.meeting);
        // Re-read the just-processed item's status for the failure toast.
        const after = itemsRef.current.find((i) => i.meeting.id === next.meeting.id);
        if (after?.status === 'error') failedTitles.push(next.meeting.title);
      }
    } finally {
      processingRef.current = false;
      dispatch({ type: 'processing-stopped' });
    }
    if (failedTitles.length > 0) {
      const who = failedTitles.length === 1 ? `'${failedTitles[0]}'` : `${failedTitles.length} meetings`;
      toast.warning(`Processing incomplete for ${who}`, {
        description: 'Open the meeting to retry, or it will retry on the next power connection.',
      });
    }
  }, [processMeeting]);
```

- [ ] **Step 6: Rewrite the trigger effects** — replace `runCheck`/`scheduleCheck`/the three power/recording effects/the immediate-process effect/`accept`/`decline`/`dismissBadge` (lines 467–596) with: a shared `refreshAndMaybeStart` that lists deferred meetings, enqueues them, and auto-starts on AC; mount + power-change wiring; and the `process-deferred-meeting-now` listener that now enqueues (never silently drops).

```ts
  const clearCheckTimer = useCallback(() => {
    if (checkTimerRef.current) {
      clearTimeout(checkTimerRef.current);
      checkTimerRef.current = null;
    }
  }, []);

  /** List deferred meetings, enqueue them, and auto-start if on AC (or forced). */
  const refreshAndMaybeStart = useCallback(
    async (opts?: { force?: boolean }) => {
      let onBattery = true;
      try {
        const power = await invoke<PowerState>('api_get_power_state');
        onBattery = power.onBattery;
      } catch (error) {
        console.warn('[deferred-backlog] power-state check failed:', error);
        onBattery = true;
      }
      let meetings: DeferredMeeting[] = [];
      try {
        meetings = await invoke<DeferredMeeting[]>('api_list_deferred_meetings');
      } catch (error) {
        console.warn('[deferred-backlog] deferred-meeting list failed:', error);
        return;
      }
      if (meetings.length === 0 && !opts?.force) return;
      dispatch({ type: 'enqueue', meetings });
      // decideAutostart only needs "is there a backlog?" — we already returned early on 0.
      if (opts?.force || decideAutostart({ backlogCount: meetings.length, onBattery, isProcessing: processingRef.current })) {
        void drain();
      }
    },
    [drain],
  );

  const scheduleRefresh = useCallback(
    (delayMs: number, opts?: { force?: boolean }) => {
      clearCheckTimer();
      checkTimerRef.current = setTimeout(() => {
        checkTimerRef.current = null;
        void refreshAndMaybeStart(opts);
      }, delayMs);
    },
    [clearCheckTimer, refreshAndMaybeStart],
  );

  // On mount: check for a backlog and auto-start if on AC (short debounce).
  useEffect(() => {
    scheduleRefresh(AUTOSTART_DEBOUNCE_MS);
    return clearCheckTimer;
  }, [scheduleRefresh, clearCheckTimer]);

  // Power→AC: re-check + auto-start (debounced). Power→battery: cancel a pending check.
  useEffect(() => {
    return safeListen<PowerSourceChangedPayload>('power-source-changed', (event) => {
      if (event.payload.onBattery) {
        clearCheckTimer();
      } else {
        scheduleRefresh(AUTOSTART_DEBOUNCE_MS);
      }
    });
  }, [scheduleRefresh, clearCheckTimer]);

  // Immediate single-meeting request (per-meeting "Process now" button / mid-recording override).
  // Enqueue + start now, regardless of power — the user asked for it explicitly.
  useEffect(() => {
    const handler = (event: Event) => {
      const meetingId = (event as CustomEvent<{ meetingId?: string }>).detail?.meetingId;
      if (meetingId) enqueueMeetingRef.current(meetingId, { force: true });
    };
    window.addEventListener('process-deferred-meeting-now', handler);
    return () => window.removeEventListener('process-deferred-meeting-now', handler);
  }, []);
```

- [ ] **Step 7: Implement `enqueueMeeting`/`stop`/`dismissDone`/`startNow` and the return** — replace the tail of the hook. `enqueueMeeting` fetches the meeting's folder/title/transcript-count (reusing the old immediate-path fetch), dispatches `enqueue`, then starts a drain if forced or on AC. Use an `enqueueMeetingRef` so the window listener (Step 6) always calls the latest.

```ts
  const enqueueMeeting = useCallback(
    async (meetingId: string, opts?: { force?: boolean }) => {
      const meeting = await storageService.getMeeting(meetingId).catch(() => null);
      const folderPath = (meeting as { folder_path?: string } | null)?.folder_path;
      if (!folderPath) {
        console.warn('[deferred-backlog] no folder_path for', meetingId);
        return;
      }
      let transcriptCount = 0;
      try {
        const page = await invoke<{ total_count: number }>('api_get_meeting_transcripts', {
          meetingId, limit: 1, offset: 0,
        });
        transcriptCount = page.total_count;
      } catch (error) {
        console.warn('[deferred-backlog] transcript count failed:', error);
      }
      const m: DeferredMeeting = {
        id: meetingId,
        title: (meeting as { title?: string } | null)?.title ?? 'meeting',
        folderPath,
        transcriptCount,
      };
      dispatch({ type: 'enqueue', meetings: [m] });
      if (opts?.force) {
        void drain();
        return;
      }
      let onBattery = true;
      try {
        onBattery = (await invoke<PowerState>('api_get_power_state')).onBattery;
      } catch { /* default to not auto-starting */ }
      if (decideAutostart({ backlogCount: 1, onBattery, isProcessing: processingRef.current })) {
        void drain();
      }
    },
    [drain],
  );

  const enqueueMeetingRef = useRef(enqueueMeeting);
  useEffect(() => { enqueueMeetingRef.current = enqueueMeeting; }, [enqueueMeeting]);

  const stop = useCallback(() => { stopRef.current = true; }, []);
  const dismissDone = useCallback(() => { dispatch({ type: 'clear-done' }); }, []);
  const startNow = useCallback(() => { void drain(); }, [drain]);

  return {
    view: selectBacklogView(state),
    enqueueMeeting: (id, opts) => { void enqueueMeeting(id, opts); },
    stop,
    dismissDone,
    startNow,
  };
```

> Note: Step 6 references `enqueueMeetingRef` which is defined in Step 7. When editing, place the `enqueueMeetingRef` definition (Step 7) BEFORE the immediate-process effect (Step 6) in the file, or keep the effect after the ref. Order the final file so every ref is defined before the effect that reads it.

- [ ] **Step 8: Remove the now-dead helpers and update the doc comment** — delete `MeetingOutcome`, `SummaryOutcome`'s `'aborted'` handling that referenced recording, `backlogProgressLabel` import, and update the top-of-file JSDoc (lines 1–23) to describe auto-start-on-AC + concurrent-with-recording + enqueue semantics. `runSummary`'s `shouldAbort` (around line 321) uses `stopRef.current`.

- [ ] **Step 9: Typecheck + file-size check**

Run: `cd frontend && pnpm lint && npx tsc --noEmit && bash ../scripts/check-file-size.sh 2>/dev/null || bash scripts/check-file-size.sh`
Expected: no type errors; `useDeferredBacklog.ts` under 800 lines (it shrinks — prompt/queue bookkeeping removed).

- [ ] **Step 10: Commit**

```bash
git add frontend/src/hooks/useDeferredBacklog.ts
git commit -m "feat(0045): decouple processing from recording; drain loop + auto-start + enqueue (WS1a/WS2)"
```

---

## Task 4: Context provider (`DeferredBacklogProvider`)

Instantiate the controller once and share it, so the three surfaces read the same state.

**Files:**
- Create: `frontend/src/contexts/DeferredBacklogProvider.tsx`

**Interfaces:**
- Consumes: `useDeferredBacklog`, `UseDeferredBacklogReturn`.
- Produces: `DeferredBacklogProvider: React.FC<{ children: React.ReactNode }>`; `useBacklog(): UseDeferredBacklogReturn`.

- [ ] **Step 1: Implement the provider**

```tsx
// frontend/src/contexts/DeferredBacklogProvider.tsx
'use client';

import { createContext, useContext, type ReactNode } from 'react';
import { useDeferredBacklog, type UseDeferredBacklogReturn } from '@/hooks/useDeferredBacklog';

const BacklogContext = createContext<UseDeferredBacklogReturn | null>(null);

/**
 * Instantiates the deferred-backlog controller ONCE and shares it (spec 0045). The controller
 * owns power watching, auto-start, and the sequential drain; every process affordance (Today
 * button, global indicator, per-meeting "Process now") reads this one instance so their state
 * is always consistent — no local set-once spinners.
 */
export function DeferredBacklogProvider({ children }: { children: ReactNode }) {
  const value = useDeferredBacklog();
  return <BacklogContext.Provider value={value}>{children}</BacklogContext.Provider>;
}

export function useBacklog(): UseDeferredBacklogReturn {
  const ctx = useContext(BacklogContext);
  if (!ctx) throw new Error('useBacklog must be used within a DeferredBacklogProvider');
  return ctx;
}
```

- [ ] **Step 2: Typecheck**

Run: `cd frontend && npx tsc --noEmit`
Expected: no errors.

- [ ] **Step 3: Commit**

```bash
git add frontend/src/contexts/DeferredBacklogProvider.tsx
git commit -m "feat(0045): DeferredBacklogProvider — one shared controller instance (WS3)"
```

---

## Task 5: Detail popover + global indicator components

**Files:**
- Create: `frontend/src/components/DeferredBacklog/BacklogDetailPopover.tsx`
- Create: `frontend/src/components/DeferredBacklog/DeferredBacklogIndicator.tsx`

**Interfaces:**
- Consumes: `useBacklog` (Task 4); `BacklogItemStatus`.
- Produces: `BacklogDetailPopover: React.FC<{ onClose?: () => void }>`; `DeferredBacklogIndicator: React.FC` (default export).

- [ ] **Step 1: Implement the per-meeting status list**

```tsx
// frontend/src/components/DeferredBacklog/BacklogDetailPopover.tsx
'use client';

import { Loader2, Check, AlertTriangle, Clock } from 'lucide-react';
import { useBacklog } from '@/contexts/DeferredBacklogProvider';
import type { BacklogItemStatus } from '@/lib/deferred-backlog';

const STATUS_LABEL: Record<BacklogItemStatus, string> = {
  waiting: 'Waiting',
  transcribing: 'Transcribing…',
  diarizing: 'Identifying speakers…',
  summarizing: 'Summarizing…',
  done: 'Done',
  error: 'Needs retry',
};

function StatusIcon({ status }: { status: BacklogItemStatus }) {
  if (status === 'done') return <Check size={14} className="text-emerald-500" aria-hidden />;
  if (status === 'error') return <AlertTriangle size={14} className="text-amber-500" aria-hidden />;
  if (status === 'waiting') return <Clock size={14} className="text-muted-foreground" aria-hidden />;
  return <Loader2 size={14} className="animate-spin text-brand" aria-hidden />;
}

/** Lists each backlog meeting by title with its live per-meeting status (spec 0045 WS3). */
export function BacklogDetailPopover({ onClose }: { onClose?: () => void }) {
  const { view, stop, startNow, dismissDone } = useBacklog();
  const hasDone = view.items.some((i) => i.status === 'done' || i.status === 'error');
  return (
    <div className="w-80 max-w-[90vw] rounded-xl border border-border bg-popover p-2 shadow-xl">
      <div className="flex items-center justify-between px-2 py-1.5">
        <span className="text-sm font-semibold text-foreground">Processing meetings</span>
        {view.processing ? (
          <button type="button" onClick={stop} className="text-xs text-muted-foreground hover:text-foreground">
            Stop
          </button>
        ) : view.pendingCount > 0 ? (
          <button type="button" onClick={startNow} className="text-xs font-semibold text-brand hover:underline">
            Process all
          </button>
        ) : null}
      </div>
      <ul className="max-h-72 overflow-y-auto">
        {view.items.length === 0 && (
          <li className="px-2 py-3 text-center text-xs text-muted-foreground">Nothing to process.</li>
        )}
        {view.items.map((item) => (
          <li key={item.meeting.id} className="flex items-center gap-2.5 px-2 py-1.5">
            <StatusIcon status={item.status} />
            <span className="min-w-0 flex-1 truncate text-sm text-foreground">{item.meeting.title}</span>
            <span className="shrink-0 text-xs text-muted-foreground">{STATUS_LABEL[item.status]}</span>
          </li>
        ))}
      </ul>
      {hasDone && (
        <div className="flex justify-end px-2 pt-1">
          <button type="button" onClick={() => { dismissDone(); onClose?.(); }} className="text-xs text-muted-foreground hover:text-foreground">
            Clear finished
          </button>
        </div>
      )}
    </div>
  );
}
```

- [ ] **Step 2: Implement the global indicator** (fixed, route-independent — replaces the nag pill; opens the popover)

```tsx
// frontend/src/components/DeferredBacklog/DeferredBacklogIndicator.tsx
'use client';

import { useState } from 'react';
import { Loader2 } from 'lucide-react';
import { useBacklog } from '@/contexts/DeferredBacklogProvider';
import { BacklogDetailPopover } from './BacklogDetailPopover';

/**
 * Global deferred-backlog indicator (spec 0045 WS3). Visible on every post-onboarding route
 * whenever a backlog exists or is processing. Compact by default; click to expand the
 * per-meeting status list. Replaces the old bottom-center prompt pill — no nagging.
 */
export default function DeferredBacklogIndicator() {
  const { view } = useBacklog();
  const [open, setOpen] = useState(false);
  const visible = view.pendingCount > 0 || view.processing || view.items.length > 0;
  if (!visible) return null;

  const label = view.processing && view.active
    ? `Processing ${view.activeOrdinal} of ${view.total} — ${view.active.meeting.title}`
    : view.pendingCount > 0
      ? `${view.pendingCount} meeting${view.pendingCount === 1 ? '' : 's'} to process`
      : 'Processing complete';

  return (
    <div className="fixed bottom-6 left-0 right-0 z-50 flex justify-center pointer-events-none">
      <div className="pointer-events-auto relative">
        {open && (
          <div className="absolute bottom-12 left-1/2 -translate-x-1/2">
            <BacklogDetailPopover onClose={() => setOpen(false)} />
          </div>
        )}
        <button
          type="button"
          onClick={() => setOpen((o) => !o)}
          aria-label="Deferred meeting processing status"
          className="flex items-center gap-2.5 rounded-2xl bg-foreground px-4 py-2.5 text-sm font-medium text-background shadow-xl max-w-[90vw]"
        >
          {view.processing && <Loader2 size={16} className="animate-spin shrink-0" aria-hidden />}
          <span className="truncate max-w-[70vw]">{label}</span>
        </button>
      </div>
    </div>
  );
}
```

- [ ] **Step 3: Typecheck + lint**

Run: `cd frontend && npx tsc --noEmit && pnpm lint`
Expected: no errors. (If `bg-popover`/`text-popover-foreground` tokens are absent, substitute `bg-card`/`text-foreground` — verify against `tailwind.config` / an existing popover, e.g. `CommandPalette`.)

- [ ] **Step 4: Commit**

```bash
git add frontend/src/components/DeferredBacklog/BacklogDetailPopover.tsx frontend/src/components/DeferredBacklog/DeferredBacklogIndicator.tsx
git commit -m "feat(0045): backlog detail popover + global indicator (WS3)"
```

---

## Task 6: Today-header "Process meetings" button

**Files:**
- Create: `frontend/src/components/DeferredBacklog/ProcessMeetingsButton.tsx`
- Modify: `frontend/src/components/Today/TodayHeader.tsx`

**Interfaces:**
- Consumes: `useBacklog` (Task 4); `Button` from `@/components/ui/button`.
- Produces: `ProcessMeetingsButton: React.FC` — renders nothing when there's no backlog.

- [ ] **Step 1: Implement the button**

```tsx
// frontend/src/components/DeferredBacklog/ProcessMeetingsButton.tsx
'use client';

import { useState } from 'react';
import { Loader2 } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { useBacklog } from '@/contexts/DeferredBacklogProvider';
import { BacklogDetailPopover } from './BacklogDetailPopover';

/**
 * Prominent "Process meetings" control for the Today header (spec 0045 WS3), mirroring the
 * Record button's slot. Shown only when a backlog exists. Idle → "Process N meetings" starts
 * the drain; running → "Processing… (k of N)" opens the status popover. State comes entirely
 * from the shared controller, so it never shows a fake spinner and survives navigation.
 */
export function ProcessMeetingsButton() {
  const { view, startNow } = useBacklog();
  const [open, setOpen] = useState(false);
  if (view.pendingCount === 0 && !view.processing) return null;

  return (
    <div className="relative">
      {view.processing ? (
        <Button variant="outline" onClick={() => setOpen((o) => !o)} className="gap-2">
          <Loader2 size={14} className="animate-spin" aria-hidden />
          Processing… ({view.activeOrdinal} of {view.total})
        </Button>
      ) : (
        <Button variant="outline" onClick={startNow} className="gap-2">
          <span className="h-1.5 w-1.5 rounded-full bg-brand" aria-hidden />
          Process {view.pendingCount} meeting{view.pendingCount === 1 ? '' : 's'}
        </Button>
      )}
      {open && (
        <div className="absolute right-0 top-11 z-50">
          <BacklogDetailPopover onClose={() => setOpen(false)} />
        </div>
      )}
    </div>
  );
}
```

- [ ] **Step 2: Render it in the header** — in `TodayHeader.tsx`, import the button and place it in the action cluster (the `<div className="flex flex-shrink-0 items-center gap-2.5">` at line 36), immediately before the New-note `<Button>` (line 56):

```tsx
import { ProcessMeetingsButton } from '@/components/DeferredBacklog/ProcessMeetingsButton';
// ...inside the action cluster, before <Button variant="outline" onClick={onNewNote} ...>:
        <ProcessMeetingsButton />
```

- [ ] **Step 3: Typecheck + lint**

Run: `cd frontend && npx tsc --noEmit && pnpm lint`
Expected: no errors.

- [ ] **Step 4: Commit**

```bash
git add frontend/src/components/DeferredBacklog/ProcessMeetingsButton.tsx frontend/src/components/Today/TodayHeader.tsx
git commit -m "feat(0045): Today-header Process-meetings button (WS3)"
```

---

## Task 7: Wire the provider into layout; retire the nag pill

**Files:**
- Modify: `frontend/src/app/layout.tsx`
- Delete: `frontend/src/components/DeferredBacklog/DeferredBacklogPrompt.tsx`

- [ ] **Step 1: Wrap the app in the provider and swap the surface** — in `layout.tsx`: add the import, wrap the post-onboarding subtree with `<DeferredBacklogProvider>` (place it inside `RecordingStateProvider` so the controller has recording context available if ever needed, and outside `SidebarProvider` is fine since the hook uses `useSidebar` — put it INSIDE `SidebarProvider`), and replace `<DeferredBacklogPrompt />` (line 285) with `<DeferredBacklogIndicator />`.

```tsx
// imports
import { DeferredBacklogProvider } from '@/contexts/DeferredBacklogProvider';
import DeferredBacklogIndicator from '@/components/DeferredBacklog/DeferredBacklogIndicator';
// remove: import DeferredBacklogPrompt from '@/components/DeferredBacklog/DeferredBacklogPrompt';
```

Wrap the subtree that contains the recording/backlog surfaces (must be INSIDE `SidebarProvider` because the controller calls `useSidebar()`, and inside `ConfigProvider` because it calls `useConfig()`):

```tsx
// Inside SidebarProvider's subtree, wrapping the block that renders GlobalRecordingBar etc.:
<DeferredBacklogProvider>
  {/* ...existing GlobalRecordingBar / indicator / modals block... */}
  <GlobalRecordingBar />
  <DeferredBacklogIndicator />   {/* was <DeferredBacklogPrompt /> */}
  <PermissionsModal />
  <ResumeRecordingPrompt />
</DeferredBacklogProvider>
```

> Verify provider ordering: `DeferredBacklogProvider` must be a descendant of `ConfigProvider`, `SidebarProvider`, and `RecordingStateProvider` (the hook's `useConfig`/`useSidebar` dependencies) and an ancestor of any component calling `useBacklog()` (the indicator here, plus Today's `ProcessMeetingsButton` and meeting-details' `TranscriptButtonGroup` — both rendered in pages nested under this layout, so this placement covers them).

- [ ] **Step 2: Delete the old prompt component**

```bash
git rm frontend/src/components/DeferredBacklog/DeferredBacklogPrompt.tsx
```

- [ ] **Step 3: Verify nothing else imports the deleted component**

Run: `cd frontend && grep -rn "DeferredBacklogPrompt" src/ && echo "STILL REFERENCED" || echo "clean"`
Expected: `clean`.

- [ ] **Step 4: Typecheck + lint + full test run**

Run: `cd frontend && npx tsc --noEmit && pnpm lint && pnpm test`
Expected: no errors; all Vitest suites pass.

- [ ] **Step 5: Commit**

```bash
git add frontend/src/app/layout.tsx
git commit -m "feat(0045): mount DeferredBacklogProvider; replace nag pill with indicator (WS3)"
```

---

## Task 8: Per-meeting "Process now" reads shared state (fixes silent no-op)

Delete the local set-once `processRequested` spinner; drive the button from the shared controller so it reflects reality, survives navigation, and enqueues (never silently drops).

**Files:**
- Modify: `frontend/src/components/MeetingDetails/TranscriptButtonGroup.tsx`

- [ ] **Step 1: Remove local state; wire the shared controller** — delete `const [processRequested, setProcessRequested] = useState(false)` (and any remaining `processRequested` references). Add the `useBacklog()` call (unconditionally, at the top of the component) and rewrite `handleProcessNow` to enqueue:

```tsx
import { useBacklog } from '@/contexts/DeferredBacklogProvider';
// ...inside the component, at top level (not in a conditional):
  const { view, enqueueMeeting } = useBacklog();

  const handleProcessNow = useCallback(() => {
    if (!meetingId) return;
    enqueueMeeting(meetingId, { force: true });
  }, [meetingId, enqueueMeeting]);
```

- [ ] **Step 2: Derive this meeting's status from the shared view** — the button re-renders on status changes because `view` comes from the shared reducer:

```tsx
  const backlogStatus =
    meetingId ? (view.items.find((i) => i.meeting.id === meetingId)?.status ?? null) : null;
  const isProcessingThis =
    backlogStatus === 'waiting' || backlogStatus === 'transcribing' ||
    backlogStatus === 'diarizing' || backlogStatus === 'summarizing';
```

- [ ] **Step 3: Update the button JSX** (the `offerProcessNow` block, lines 205–221) to use `isProcessingThis` and a "Queued" state:

```tsx
        {offerProcessNow && (
          <Button
            size="xs"
            variant="outline"
            className="bg-brand/10 hover:bg-brand/20 border-brand/30 text-brand gap-1.5"
            onClick={handleProcessNow}
            disabled={isProcessingThis}
            title={
              isProcessingThis
                ? 'Processing this meeting — see status in the indicator'
                : 'This meeting was recorded in low-power mode — transcribe, identify speakers, and summarize it now'
            }
          >
            {isProcessingThis && <Loader2 className="animate-spin" size={16} />}
            <span>
              {backlogStatus === 'waiting'
                ? 'Queued'
                : isProcessingThis
                  ? 'Processing…'
                  : 'Process now'}
            </span>
          </Button>
        )}
```

> `useBacklog()` is a hook — call it unconditionally at the top of the component (not inside a conditional). The `meetingId ? … : null` guard applies only to the derived value, not the hook call.

- [ ] **Step 4: Verify the meeting-details page is under the provider** — `TranscriptButtonGroup` renders inside the meeting-details route, which is nested under the root layout wrapped in Task 7. Confirm no separate render tree bypasses it.

Run: `cd frontend && npx tsc --noEmit && pnpm lint`
Expected: no errors.

- [ ] **Step 5: Commit**

```bash
git add frontend/src/components/MeetingDetails/TranscriptButtonGroup.tsx
git commit -m "fix(0045): per-meeting Process now reads shared controller state — no more fake spinner (WS3, symptom 7)"
```

---

## Task 9: Context-aware unprocessed empty state (kill "Welcome to Vinyl!")

**Files:**
- Modify: `frontend/src/lib/processing-mode.ts` (+ its test file if present)
- Modify: `frontend/src/components/VirtualizedTranscriptView.tsx`
- Modify: `frontend/src/components/MeetingDetails/TranscriptPanel.tsx`

**Interfaces:**
- Produces (processing-mode.ts): `isUnprocessedRecording(input: { hasTranscripts: boolean; isDeferred: boolean; audioAvailable: boolean }): boolean`.
- VirtualizedTranscriptView gains optional props `unprocessed?: boolean` and `onProcessNow?: () => void`.

- [ ] **Step 1: Write the failing test for the helper**

```ts
// append to frontend/src/lib/__tests__/processing-mode.test.ts (create if absent)
import { describe, it, expect } from 'vitest';
import { isUnprocessedRecording } from '@/lib/processing-mode';

describe('isUnprocessedRecording', () => {
  it('true for a deferred meeting with audio and no transcript', () => {
    expect(isUnprocessedRecording({ hasTranscripts: false, isDeferred: true, audioAvailable: true })).toBe(true);
  });
  it('false once a transcript exists', () => {
    expect(isUnprocessedRecording({ hasTranscripts: true, isDeferred: true, audioAvailable: true })).toBe(false);
  });
  it('false when no audio on disk (genuinely empty / notes-only)', () => {
    expect(isUnprocessedRecording({ hasTranscripts: false, isDeferred: true, audioAvailable: false })).toBe(false);
  });
  it('false for a normal non-deferred empty view', () => {
    expect(isUnprocessedRecording({ hasTranscripts: false, isDeferred: false, audioAvailable: false })).toBe(false);
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd frontend && pnpm test -- processing-mode`
Expected: FAIL — `isUnprocessedRecording` not exported.

- [ ] **Step 3: Implement the helper** (append to `processing-mode.ts`)

```ts
/**
 * A finished recording that has audio on disk but no transcript yet — i.e. recorded in
 * low-power/deferred mode and not processed. Drives the transcript empty state so it reads
 * "not processed yet" instead of the generic welcome copy (spec 0045 WS4).
 */
export function isUnprocessedRecording(input: {
  hasTranscripts: boolean;
  isDeferred: boolean;
  audioAvailable: boolean;
}): boolean {
  return !input.hasTranscripts && input.isDeferred && input.audioAvailable;
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd frontend && pnpm test -- processing-mode`
Expected: PASS.

- [ ] **Step 5: Add props + branch to VirtualizedTranscriptView** — extend the props interface (line 71) with `unprocessed?: boolean;` and `onProcessNow?: () => void;`, destructure with defaults (line 335 area: `unprocessed = false, onProcessNow,`), and replace the not-recording empty-state branch (lines 756–760):

```tsx
                    ) : unprocessed ? (
                        <>
                            <p className="font-display text-lg font-semibold">This recording hasn&apos;t been processed yet</p>
                            <p className="text-xs mt-1">Transcribe, identify speakers, and summarize it now.</p>
                            {onProcessNow && (
                                <button
                                    type="button"
                                    onClick={onProcessNow}
                                    className="mt-3 inline-flex items-center rounded-lg bg-brand px-3 py-1.5 text-xs font-semibold text-brand-foreground hover:bg-brand/90"
                                >
                                    Process now
                                </button>
                            )}
                        </>
                    ) : (
                        <>
                            <p className="font-display text-lg font-semibold">Welcome to Vinyl!</p>
                            <p className="text-xs mt-1">Start recording to see live transcription</p>
                        </>
                    )}
```

> If `brand-foreground` isn't a defined token, use `text-white` (match the `bg-record text-record-foreground` / `variant="blue"` convention already in the header).

- [ ] **Step 6: Compute and pass `unprocessed` from the meeting-details panel** — in `MeetingDetails/TranscriptPanel.tsx`, the panel already knows `isRecording`, `meetingId`, and renders `TranscriptButtonGroup` (which probes `processingMode`/`audioAvailable`). Lift or re-probe those, compute `unprocessed`, and pass it plus an `onProcessNow` to `VirtualizedTranscriptView` (line 253). If the panel doesn't already have `isDeferred`/`audioAvailable`, read them the same way `TranscriptButtonGroup` does (`api_get_meeting_processing_mode` → `=== 'defer'`; `api_meeting_audio_available`):

```tsx
import { isUnprocessedRecording } from '@/lib/processing-mode';
import { useBacklog } from '@/contexts/DeferredBacklogProvider';
// ...compute (reuse existing probes if present in this panel; otherwise add them):
  const unprocessed = isUnprocessedRecording({
    hasTranscripts: convertedSegments.length > 0,
    isDeferred,          // processingMode === 'defer'
    audioAvailable,      // api_meeting_audio_available
  });
  const { enqueueMeeting } = useBacklog();
// ...pass to the view (line ~253):
        <VirtualizedTranscriptView
          // ...existing props...
          isRecording={isRecording}
          unprocessed={!isRecording && unprocessed}
          onProcessNow={meetingId ? () => enqueueMeeting(meetingId, { force: true }) : undefined}
        />
```

> To avoid duplicating the `processingMode`/`audioAvailable` probes across `TranscriptPanel` and `TranscriptButtonGroup`, prefer lifting them into the panel and passing down as props IF that's a small change; otherwise re-probe (they're cheap IPC reads). Keep the panel under 800 lines.

- [ ] **Step 7: Typecheck + lint + tests**

Run: `cd frontend && npx tsc --noEmit && pnpm lint && pnpm test`
Expected: no errors; all suites pass.

- [ ] **Step 8: Commit**

```bash
git add frontend/src/lib/processing-mode.ts frontend/src/lib/__tests__/processing-mode.test.ts frontend/src/components/VirtualizedTranscriptView.tsx frontend/src/components/MeetingDetails/TranscriptPanel.tsx
git commit -m "feat(0045): context-aware unprocessed empty state — replaces 'Welcome to Vinyl!' (WS4)"
```

---

## Task 10: Backend — serialize STT decode for safe concurrent live + batch

`unload_engine_after_batch` already skips unloading while recording (`common.rs:61`, under `acquire_engine_lifecycle_lock`). The remaining risk once the frontend recording-gate is gone: `WhisperEngine::transcribe_audio` / `ParakeetEngine::transcribe_audio` run whisper.cpp/Parakeet inference while holding only a `RwLock::read` on the shared context (`whisper_engine.rs:659`), so a live frame and a batch segment can decode on the **same** context concurrently — not safe. Add a process-wide inference mutex so decode calls take turns (batch waits behind each live frame). This honors "always runs" without doubling GPU memory.

**Files:**
- Create: `frontend/src-tauri/src/audio/stt_lock.rs`
- Modify: `frontend/src-tauri/src/audio/mod.rs` (declare the module)
- Modify: `frontend/src-tauri/src/whisper_engine/whisper_engine.rs` (guard the decode in `transcribe_audio` + `transcribe_audio_with_confidence`)
- Modify: `frontend/src-tauri/src/parakeet_engine/parakeet_engine.rs` (guard the decode in `transcribe_audio`)

**Interfaces:**
- Produces: `crate::audio::stt_lock::acquire_inference_lock().await -> tokio::sync::MutexGuard<'static, ()>`.

- [ ] **Step 1: Write the failing test** — a Rust test proving the lock serializes (only one holder at a time).

```rust
// frontend/src-tauri/src/audio/stt_lock.rs  (test module at the bottom)
#[cfg(test)]
mod tests {
    use super::acquire_inference_lock;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn only_one_holder_at_a_time() {
        let concurrent = Arc::new(AtomicUsize::new(0));
        let max = Arc::new(AtomicUsize::new(0));
        let mut handles = Vec::new();
        for _ in 0..8 {
            let c = concurrent.clone();
            let m = max.clone();
            handles.push(tokio::spawn(async move {
                let _g = acquire_inference_lock().await;
                let now = c.fetch_add(1, Ordering::SeqCst) + 1;
                m.fetch_max(now, Ordering::SeqCst);
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
                c.fetch_sub(1, Ordering::SeqCst);
            }));
        }
        for h in handles { h.await.unwrap(); }
        assert_eq!(max.load(Ordering::SeqCst), 1, "inference lock must serialize decode");
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd frontend/src-tauri && source ~/.cargo/env && cargo test --features metal stt_lock 2>&1 | tail -20`
Expected: FAIL — module/function doesn't exist (compile error).

- [ ] **Step 3: Implement the lock**

```rust
// frontend/src-tauri/src/audio/stt_lock.rs
//! Process-wide STT inference lock (spec 0045 WS1b).
//!
//! Live transcription and batch retranscription share the same global whisper/Parakeet
//! engine context (`WHISPER_ENGINE` / `PARAKEET_ENGINE`). Once deferred processing is allowed
//! to run concurrently with a live recording, two decode calls could hit the same context at
//! once — which whisper.cpp/Parakeet is not safe against. This mutex serializes the actual
//! decode so they take turns (the batch segment waits behind each live frame). Memory-cheap
//! alternative to a second resident engine; the small added live-frame latency is the
//! accepted contention trade-off (owner decision, spec 0045).

use tokio::sync::Mutex;

static INFERENCE_LOCK: Mutex<()> = Mutex::const_new(());

/// Acquire exclusive access to STT decode. Held for the duration of one inference call.
pub async fn acquire_inference_lock() -> tokio::sync::MutexGuard<'static, ()> {
    INFERENCE_LOCK.lock().await
}
```

Declare the module in `frontend/src-tauri/src/audio/mod.rs` (add near the other `pub mod`/`mod` lines, matching the file's convention):

```rust
pub mod stt_lock;
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd frontend/src-tauri && source ~/.cargo/env && cargo test --features metal stt_lock 2>&1 | tail -20`
Expected: PASS (`only_one_holder_at_a_time`).

- [ ] **Step 5: Guard the whisper decode** — in `whisper_engine.rs::transcribe_audio` (line 654) and `transcribe_audio_with_confidence` (line 532), acquire the lock for the decode critical section. Take it right after obtaining the context read-guard, before running inference:

```rust
    pub async fn transcribe_audio(
        &self,
        audio_data: Vec<f32>,
        language: Option<String>,
    ) -> Result<String> {
        let _inference = crate::audio::stt_lock::acquire_inference_lock().await;
        let ctx_lock = self.current_context.read().await;
        // ...unchanged body...
    }
```

Apply the identical `let _inference = …;` as the first line of `transcribe_audio_with_confidence`.

- [ ] **Step 6: Guard the Parakeet decode** — in `parakeet_engine.rs::transcribe_audio` (line 481), add the same first line:

```rust
    pub async fn transcribe_audio(&self, audio_data: Vec<f32>) -> Result<String> {
        let _inference = crate::audio::stt_lock::acquire_inference_lock().await;
        // ...unchanged body...
    }
```

- [ ] **Step 7: Build + clippy + the STT/pipeline test subset**

Run: `cd frontend/src-tauri && source ~/.cargo/env && cargo clippy --features metal 2>&1 | tail -20 && cargo test --features metal --test transcription_engine --test pipeline_integration 2>&1 | tail -30`
Expected: clippy clean (only pre-existing warnings, if any); transcription/pipeline tests pass. (Note the pre-existing `vad_filter` failure is exempt and not in this subset.)

- [ ] **Step 8: Commit**

```bash
git add frontend/src-tauri/src/audio/stt_lock.rs frontend/src-tauri/src/audio/mod.rs frontend/src-tauri/src/whisper_engine/whisper_engine.rs frontend/src-tauri/src/parakeet_engine/parakeet_engine.rs
git commit -m "fix(0045): serialize STT decode so live + batch transcription can't race (WS1b)"
```

---

## Task 11: Reproduction guard — immediate process is never silently dropped

A focused Vitest that reproduces symptom 7 at the reducer/selector level: a "Process now" (enqueue) while a drain is conceptually running still lands the meeting in the backlog as `waiting` (to be drained), never dropped.

**Files:**
- Test: `frontend/src/lib/__tests__/deferred-backlog.test.ts` (add a describe block)

- [ ] **Step 1: Write the test**

```ts
describe('enqueue is never a silent no-op (spec 0045 symptom 7)', () => {
  it('adds a requested meeting even when others are mid-flight', () => {
    let s = backlogReducer(initialBacklogState, { type: 'enqueue', meetings: [m('a')] });
    s = backlogReducer(s, { type: 'processing-started' });
    s = backlogReducer(s, { type: 'set-status', meetingId: 'a', status: 'transcribing' });
    // User clicks "Process now" on a different meeting while 'a' is processing:
    s = backlogReducer(s, { type: 'enqueue', meetings: [m('b')] });
    expect(backlogItemStatus(s, 'b')).toBe('waiting'); // queued, not dropped
    expect(firstWaiting(s)?.meeting.id).toBe('b');      // the drain loop will pick it up
  });
});
```

- [ ] **Step 2: Run it**

Run: `cd frontend && pnpm test -- deferred-backlog`
Expected: PASS (the model already supports this from Task 2 — this locks the behavior against regression).

- [ ] **Step 3: Commit**

```bash
git add frontend/src/lib/__tests__/deferred-backlog.test.ts
git commit -m "test(0045): enqueue-while-busy is never silently dropped (symptom 7 guard)"
```

---

## Task 12: Full verification + manual smoke

- [ ] **Step 1: Frontend gate**

Run: `cd frontend && pnpm lint && pnpm test`
Expected: clean; all suites pass.

- [ ] **Step 2: Rust gate**

Run: `cd frontend/src-tauri && source ~/.cargo/env && cargo check --features metal && cargo clippy --features metal && cargo test --features metal --test transcription_engine --test pipeline_integration --test db_lifecycle`
Expected: clean (pre-existing `vad_filter` failure exempt; run it separately only to confirm it's unchanged).

- [ ] **Step 3: File-size ratchet**

Run: `cd frontend && bash scripts/check-file-size.sh || (cd .. && bash scripts/check-file-size.sh)`
Expected: PASS — no touched file over 800 lines; `useDeferredBacklog.ts` shrank.

- [ ] **Step 4: App launches**

Run: `./clean_run.sh` (from repo root, sidecar present) — app boots.

- [ ] **Step 5: Manual smoke (the owner's scenario)**
  1. Record meeting A on battery (deferred, record-only). Stop.
  2. Start recording meeting B; while B records, reconnect to power. → within ~5s, A auto-starts; the Today indicator/global chip shows "Processing 1 of 1 — A", and B's recording is **not** aborted; A completes to ✓ Done.
  3. Open A before it processes (on battery) → transcript pane reads "This recording hasn't been processed yet" with a working **Process now** (enqueues + starts; on battery only when clicked).
  4. Click "Process now"; navigate away and back → the button still shows the real in-progress status (no fake spinner, not lost).
  5. On battery with a backlog → no auto-start; the Today "Process N meetings" button appears and works when clicked.

- [ ] **Step 6: Update the spec status + INDEX; final commit**

Set spec `0045` Status to "Implemented (pending owner smoke)" and the INDEX row likewise.

```bash
git add specs/0045-1.11-feedback-batch.md specs/INDEX.md
git commit -m "docs(0045): mark implemented pending owner smoke"
```

---

## Self-Review Notes (coverage map)

- **Spec symptom 1 (inconsistent reconnect prompt):** Tasks 1+3 — auto-start on AC with a 5s debounce; the recording bail is gone (`refreshAndMaybeStart` runs regardless of recording).
- **Symptom 2 (which meetings / when done):** Tasks 2+5 — per-meeting titles + persistent ✓ Done in the popover; indicator shows "k of N — title".
- **Symptom 3 (repeated prompts):** Task 3 — no prompt at all; passive indicator + auto-start.
- **Symptom 4 (doesn't finish / blocked while recording):** Task 3 — all `isRecording` gating removed; drain no longer aborts on record.
- **Symptom 5 (no top-level control):** Task 6 — Today-header button; Task 5 — global indicator.
- **Symptom 6 ("Welcome to Vinyl!"):** Task 9 — `isUnprocessedRecording` + context-aware empty state.
- **Symptom 7 (silent-no-op button):** Tasks 8 (controller-driven button, no local spinner, enqueue) + 2/11 (enqueue never dropped) + 10 (un-wedgeable via serialized decode; unload already recording-guarded).
- **Concurrency safety (WS1b):** Task 10 — serialized decode; verified existing unload guard.
- **File-size ratchet:** new logic in `backlog-autostart.ts` + `deferred-backlog.ts`; `useDeferredBacklog.ts` shrinks (Task 3 Step 9 / Task 12 Step 3).
