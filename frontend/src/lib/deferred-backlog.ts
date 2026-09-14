/**
 * Deferred-backlog per-meeting status model (deferred-processing UX spec §0045).
 *
 * When a meeting recorded in deferred (record-only) mode still needs transcription,
 * diarization, or summarization, it's tracked here as a `BacklogItem` keyed by meeting
 * id. Every UI surface — the Today button, the global processing indicator, and the
 * per-meeting "Process now" button — reads from the same `BacklogState` via
 * `selectBacklogView` / `backlogItemStatus` / `firstWaiting`, so they never disagree
 * about what's pending or what's currently running.
 *
 * This module holds the PURE decisions so they're unit-testable without Tauri, timers,
 * or React:
 *  - `needsRetranscription`: whether a queued meeting needs a transcription pass.
 *  - `backlogReducer` + the selectors below: the per-meeting status state machine.
 *  - The async work (retranscribe → diarize → summarize) lives in `useDeferredBacklog`,
 *    which drives this reducer.
 */

import { SPARSE_TRANSCRIPT_SEGMENTS } from '@/lib/deferred-transcription';

/** A meeting awaiting deferred processing (`api_list_deferred_meetings` payload). */
export interface DeferredMeeting {
  id: string;
  title: string;
  folderPath: string;
  transcriptCount: number;
}

/**
 * Whether a queued meeting needs a (re)transcription pass before diarize/summarize.
 * True when the transcript is sparse/absent OR the meeting was explicitly deferred
 * (record-only), whose pre-override audio has no transcript to build on.
 */
export function needsRetranscription(
  transcriptCount: number,
  explicitlyDeferred: boolean,
): boolean {
  return transcriptCount < SPARSE_TRANSCRIPT_SEGMENTS || explicitlyDeferred;
}

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
 * user can see what finished) plus whether a drain is currently running. Keyed by meeting
 * id so every surface — Today button, global indicator, per-meeting button — reads one truth.
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
  | { type: 'requeue-active' }
  | { type: 'clear-done' }
  | { type: 'reset' };

const TERMINAL: BacklogItemStatus[] = ['done', 'error'];

/**
 * Pure enqueue-merge: append meetings not yet tracked as `waiting`, reset a previously
 * finished/failed (terminal) meeting back to `waiting` if asked for again, and leave a
 * meeting that's already waiting/active untouched (no duplicate, no silent drop).
 *
 * Exported so the hook can seed its `itemsRef` mirror synchronously with the SAME merge the
 * reducer commits — the drain loop's kickoff reads the ref before React commits the dispatch.
 */
export function enqueueItems(items: BacklogItem[], meetings: DeferredMeeting[]): BacklogItem[] {
  const next = [...items];
  for (const meeting of meetings) {
    const existing = next.findIndex((i) => i.meeting.id === meeting.id);
    if (existing === -1) {
      next.push({ meeting, status: 'waiting' });
    } else if (TERMINAL.includes(next[existing].status)) {
      // A previously finished/failed meeting was asked for again — requeue it.
      next[existing] = { meeting, status: 'waiting' };
    }
    // else: already waiting/active — leave it (no duplicate, no silent drop).
  }
  return next;
}

export function backlogReducer(state: BacklogState, action: BacklogAction): BacklogState {
  switch (action.type) {
    case 'enqueue':
      return { ...state, items: enqueueItems(state.items, action.meetings) };
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
    case 'requeue-active':
      // A user Stop leaves the active meeting mid-flight in a non-terminal, non-waiting
      // status. Return any such item to 'waiting' so it's re-runnable; leave
      // waiting/done/error untouched.
      return {
        ...state,
        items: state.items.map((i) =>
          ACTIVE_STATES.includes(i.status) ? { ...i, status: 'waiting' } : i,
        ),
      };
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

/**
 * True when the backlog currently OWNS this meeting's processing — it is queued or in
 * one of the pipeline steps, so something is (or is about to be) running for it.
 *
 * spec 0051 final review, Finding 1: the stop path writes `processing_mode='defer'`
 * BEFORE handing a 'process-now' meeting to the backlog, so the marker alone can no
 * longer tell "waiting for a later launch / AC" apart from "being processed right now".
 * Every surface that reads the marker must qualify it with this. Shared so the
 * meeting-details auto-summary gate and TranscriptButtonGroup can't drift apart.
 */
export function isMeetingInFlight(
  items: BacklogItem[],
  meetingId: string | null | undefined,
): boolean {
  if (!meetingId) return false;
  const status = items.find((i) => i.meeting.id === meetingId)?.status;
  return status !== undefined && !TERMINAL.includes(status);
}

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
