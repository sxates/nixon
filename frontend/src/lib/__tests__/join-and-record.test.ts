import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';

// specs/0029 WS2.1 — the pending-join race. `joinAndRecord` must stash the calendar
// identity BEFORE opening Zoom and BEFORE the awaited `api_create_meeting`, because
// opening Zoom trips the backend meeting-detected prompt: any start path that
// wins during the create/delay window consumes the stash, and a missing stash minted
// a date-stamped, attendee-less row. The SQLite id is reconciled into the stash when
// the create resolves; a consume that raced the create awaits it via
// `waitForPendingJoinCreate`. Tauri IPC is mocked; the real sessionStorage-backed
// stash helpers run.

const { invokeMock, toastError } = vi.hoisted(() => ({
  invokeMock: vi.fn(),
  toastError: vi.fn(),
}));

vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }));
vi.mock('sonner', () => ({
  toast: Object.assign(vi.fn(), {
    error: toastError,
    info: vi.fn(),
    success: vi.fn(),
    warning: vi.fn(),
    dismiss: vi.fn(),
  }),
}));

import {
  joinAndRecord,
  openZoomMeeting,
  peekPendingJoinMeeting,
  consumePendingJoinMeeting,
  waitForPendingJoinCreate,
  JOIN_AND_RECORD_DELAY_MS,
} from '@/lib/calendar';

const EVENT = {
  id: 'evt-1',
  title: 'Weekly Sync',
  zoomUrl: 'https://zoom.us/j/1234567890?pwd=abc',
  startsAt: '2026-07-01T17:00:00Z',
};

type Deferred = { resolve: (v: unknown) => void; reject: (e: unknown) => void };

/** Route invoke: open succeeds; api_create_meeting is caller-controlled (deferred). */
function routeInvoke({
  onOpen,
  createDeferred,
}: {
  onOpen?: (url: string) => void;
  createDeferred?: Deferred[];
} = {}) {
  invokeMock.mockImplementation((cmd: string, args?: Record<string, unknown>) => {
    switch (cmd) {
      case 'open_external_url':
        onOpen?.(args?.url as string);
        return Promise.resolve();
      case 'api_create_meeting':
        if (createDeferred) {
          return new Promise((resolve, reject) => {
            createDeferred.push({ resolve, reject });
          });
        }
        return Promise.resolve({ meeting_id: 'meeting-cal' });
      default:
        return Promise.resolve(undefined);
    }
  });
}

function openCalls(): string[] {
  return invokeMock.mock.calls
    .filter((c) => c[0] === 'open_external_url')
    .map((c) => (c[1] as { url: string }).url);
}

describe('joinAndRecord — stash-before-open ordering (WS2.1)', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    sessionStorage.clear();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it('arms the pending-join stash before opening Zoom and before the create resolves', async () => {
    const stashArmedAtOpen: boolean[] = [];
    const deferred: Deferred[] = [];
    routeInvoke({
      onOpen: () => {
        stashArmedAtOpen.push(peekPendingJoinMeeting()?.calendarEventId === EVENT.id);
      },
      createDeferred: deferred,
    });

    const done = joinAndRecord(EVENT, false, vi.fn());

    // Synchronously after invocation — with the create still in flight — any racing
    // start path already sees the full calendar identity (id pending).
    expect(peekPendingJoinMeeting()).toEqual({
      id: null,
      title: 'Weekly Sync',
      calendarEventId: 'evt-1',
      startsAt: '2026-07-01T17:00:00Z',
      // specs/0036: series key is threaded through the stash (null when the event has none).
      calendarSeriesKey: null,
    });
    // Zoom was opened via the deep link, and the stash existed at open time.
    expect(openCalls()).toEqual(['zoommtg://zoom.us/join?confno=1234567890&pwd=abc']);
    expect(stashArmedAtOpen).toEqual([true]);

    deferred[0].resolve({ meeting_id: 'meeting-cal' });
    await done;

    // The SQLite id was reconciled into the still-armed stash, and the roster seeded.
    expect(peekPendingJoinMeeting()?.id).toBe('meeting-cal');
    expect(invokeMock).toHaveBeenCalledWith('api_get_meeting_participants', {
      meetingId: 'meeting-cal',
    });
  });

  it('lets a consume that raced the create await the id, without resurrecting the stash', async () => {
    const deferred: Deferred[] = [];
    routeInvoke({ createDeferred: deferred });

    const done = joinAndRecord(EVENT, false, vi.fn());

    // A racing start path (zoom-detected toast / tray / sidebar) consumes the stash
    // while the create is still in flight — it gets the identity, id pending…
    const consumed = consumePendingJoinMeeting();
    expect(consumed?.calendarEventId).toBe('evt-1');
    expect(consumed?.id).toBeNull();

    // …and waits for the in-flight create instead of minting its own row.
    const waited = waitForPendingJoinCreate();
    deferred[0].resolve({ meeting_id: 'meeting-cal' });
    await expect(waited).resolves.toBe('meeting-cal');

    await done;
    // Reconciliation must NOT resurrect the consumed stash (it would leak into the
    // next unrelated recording).
    expect(peekPendingJoinMeeting()).toBeNull();
  });

  it('keeps a null-id stash on create failure and still schedules the delayed start', async () => {
    vi.useFakeTimers();
    invokeMock.mockImplementation((cmd: string) =>
      cmd === 'api_create_meeting'
        ? Promise.reject(new Error('db down'))
        : Promise.resolve(undefined),
    );
    const startRecording = vi.fn();

    await joinAndRecord(EVENT, false, startRecording);

    // Consume falls back gracefully: identity intact, id null → the recorder creates
    // the calendar-linked row itself.
    expect(peekPendingJoinMeeting()).toEqual({
      id: null,
      title: 'Weekly Sync',
      calendarEventId: 'evt-1',
      startsAt: '2026-07-01T17:00:00Z',
      // specs/0036: series key is threaded through the stash (null when the event has none).
      calendarSeriesKey: null,
    });
    await expect(waitForPendingJoinCreate()).resolves.toBeNull();

    expect(startRecording).not.toHaveBeenCalled();
    vi.advanceTimersByTime(JOIN_AND_RECORD_DELAY_MS);
    expect(startRecording).toHaveBeenCalledTimes(1);
  });

  it('double-tap on an armed event reopens Zoom but creates no second row', async () => {
    routeInvoke();
    await joinAndRecord(EVENT, false, vi.fn());
    invokeMock.mockClear();

    routeInvoke();
    await joinAndRecord(EVENT, false, vi.fn());

    expect(openCalls()).toHaveLength(1);
    expect(invokeMock.mock.calls.filter((c) => c[0] === 'api_create_meeting')).toHaveLength(0);
  });

  it('when already recording, only opens Zoom (no stash, no row)', async () => {
    routeInvoke();
    await joinAndRecord(EVENT, true, vi.fn());

    expect(openCalls()).toHaveLength(1);
    expect(peekPendingJoinMeeting()).toBeNull();
    expect(invokeMock.mock.calls.filter((c) => c[0] === 'api_create_meeting')).toHaveLength(0);
  });
});

describe('openZoomMeeting — deep-link fallback (WS1.1)', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('falls back to the https URL when the deep link is rejected', async () => {
    invokeMock.mockImplementation((_cmd: string, args?: Record<string, unknown>) =>
      String(args?.url).startsWith('zoommtg://')
        ? Promise.reject(new Error('scheme not allowed'))
        : Promise.resolve(),
    );

    await openZoomMeeting(EVENT.zoomUrl);

    expect(openCalls()).toEqual([
      'zoommtg://zoom.us/join?confno=1234567890&pwd=abc',
      EVENT.zoomUrl,
    ]);
    expect(toastError).not.toHaveBeenCalled();
  });

  it('surfaces a toast when both the deep link and the https fallback fail', async () => {
    invokeMock.mockRejectedValue(new Error('opener broken'));

    await openZoomMeeting(EVENT.zoomUrl);

    expect(openCalls()).toHaveLength(2);
    expect(toastError).toHaveBeenCalledWith('Could not open the meeting link', expect.anything());
  });
});
