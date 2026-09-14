import { describe, it, expect } from 'vitest';
import {
  backlogReducer,
  enqueueItems,
  initialBacklogState,
  selectBacklogView,
  backlogItemStatus,
  firstWaiting,
  needsRetranscription,
  isMeetingInFlight,
  type BacklogItem,
  type BacklogState,
  type DeferredMeeting,
} from '@/lib/deferred-backlog';
import { SPARSE_TRANSCRIPT_SEGMENTS } from '@/lib/deferred-transcription';

// deferred-processing UX spec (0045) — the per-meeting backlog status model consumed by
// the Today button, global indicator, and per-meeting button. Pure so it's testable
// without Tauri, timers, or React.

const m = (id: string, title = id): DeferredMeeting => ({
  id, title, folderPath: `/rec/${id}`, transcriptCount: 0,
});

describe('needsRetranscription', () => {
  it('retranscribes a sparse-transcript meeting', () => {
    expect(needsRetranscription(0, false)).toBe(true);
    expect(needsRetranscription(SPARSE_TRANSCRIPT_SEGMENTS - 1, false)).toBe(true);
  });

  it('retranscribes an explicitly-deferred meeting even with a fuller transcript', () => {
    expect(needsRetranscription(500, true)).toBe(true);
  });

  it('skips retranscription for a well-transcribed, non-deferred meeting', () => {
    expect(needsRetranscription(SPARSE_TRANSCRIPT_SEGMENTS, false)).toBe(false);
    expect(needsRetranscription(500, false)).toBe(false);
  });
});

describe('enqueueItems', () => {
  const item = (id: string, status: BacklogItem['status']): BacklogItem => ({
    meeting: m(id),
    status,
  });

  it('appends meetings not yet tracked, as waiting', () => {
    const next = enqueueItems([item('a', 'waiting')], [m('b'), m('c')]);
    expect(next.map((i) => i.meeting.id)).toEqual(['a', 'b', 'c']);
    expect(next.every((i) => i.status === 'waiting')).toBe(true);
  });

  it('de-dupes a still-waiting/active meeting (no duplicate, status untouched)', () => {
    const next = enqueueItems([item('a', 'transcribing'), item('b', 'waiting')], [m('a'), m('b')]);
    expect(next.map((i) => i.meeting.id)).toEqual(['a', 'b']);
    expect(next.find((i) => i.meeting.id === 'a')?.status).toBe('transcribing');
    expect(next.find((i) => i.meeting.id === 'b')?.status).toBe('waiting');
  });

  it('resets a terminal (done/error) meeting back to waiting when re-enqueued', () => {
    const next = enqueueItems([item('a', 'done'), item('b', 'error')], [m('a'), m('b')]);
    expect(next.map((i) => i.status)).toEqual(['waiting', 'waiting']);
  });

  it('does not mutate the input array', () => {
    const input = [item('a', 'waiting')];
    const next = enqueueItems(input, [m('b')]);
    expect(input).toHaveLength(1);
    expect(next).toHaveLength(2);
  });
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

  it('requeue-active resets active items to waiting; leaves waiting/terminal untouched', () => {
    let s = backlogReducer(initialBacklogState, {
      type: 'enqueue',
      meetings: [m('a'), m('b'), m('c')],
    });
    s = backlogReducer(s, { type: 'set-status', meetingId: 'a', status: 'transcribing' });
    s = backlogReducer(s, { type: 'set-status', meetingId: 'b', status: 'done' });
    // 'c' stays 'waiting'.
    s = backlogReducer(s, { type: 'requeue-active' });
    expect(backlogItemStatus(s, 'a')).toBe('waiting'); // active → waiting
    expect(backlogItemStatus(s, 'b')).toBe('done'); // terminal untouched
    expect(backlogItemStatus(s, 'c')).toBe('waiting'); // waiting untouched
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

// spec 0051 final review, Finding 1: "does the backlog already own this meeting?" is
// asked by the meeting-details auto-summary gate AND TranscriptButtonGroup. One shared
// predicate so a 'defer' marker is never read as "waiting" while a pipeline is running.
describe('isMeetingInFlight', () => {
  it('is true for a queued meeting and for every pipeline step', () => {
    let s = backlogReducer(initialBacklogState, { type: 'enqueue', meetings: [m('a')] });
    expect(isMeetingInFlight(s.items, 'a')).toBe(true); // waiting
    for (const status of ['transcribing', 'diarizing', 'summarizing'] as const) {
      s = backlogReducer(s, { type: 'set-status', meetingId: 'a', status });
      expect(isMeetingInFlight(s.items, 'a')).toBe(true);
    }
  });

  it('is false once the meeting reaches a terminal status', () => {
    let s = backlogReducer(initialBacklogState, { type: 'enqueue', meetings: [m('a'), m('b')] });
    s = backlogReducer(s, { type: 'set-status', meetingId: 'a', status: 'done' });
    s = backlogReducer(s, { type: 'set-status', meetingId: 'b', status: 'error' });
    expect(isMeetingInFlight(s.items, 'a')).toBe(false);
    expect(isMeetingInFlight(s.items, 'b')).toBe(false);
  });

  it('is false for an untracked meeting and for a missing id', () => {
    const s = backlogReducer(initialBacklogState, { type: 'enqueue', meetings: [m('a')] });
    expect(isMeetingInFlight(s.items, 'nope')).toBe(false);
    expect(isMeetingInFlight(s.items, null)).toBe(false);
    expect(isMeetingInFlight([], 'a')).toBe(false);
  });
});
