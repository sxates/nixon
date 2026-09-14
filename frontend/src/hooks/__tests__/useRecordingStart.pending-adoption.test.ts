import { describe, it, expect, beforeEach, vi } from 'vitest';
import { renderHook, act } from '@testing-library/react';

// specs/0029 WS2.1 — consume-with-pending adoption. The pending Join & Record must
// survive every ordering of the start race:
//   - a consume that catches the pre-create still in flight AWAITS its id instead of
//     minting a duplicate/date-stamped row;
//   - the meetingCreatedRef early-return (another start path won the create) ADOPTS
//     the calendar-linked row instead of silently discarding the pending join;
//   - when no calendar row can be resolved, the racing row is at least re-titled
//     with the event title (never left date-stamped when the identity is known).
//
// Mock seams mirror useRecordingStart.test.ts; lib/calendar is intentionally NOT
// mocked so the real sessionStorage stash + in-flight-create slot run.

const {
  invokeMock,
  setCurrentMeeting,
  refetchMeetings,
  startRecordingWithDevices,
  clearTranscripts,
  setMeetingTitle,
  setIsMeetingActive,
  setActiveRecordingMeetingId,
  setStatus,
} = vi.hoisted(() => ({
  invokeMock: vi.fn(),
  setCurrentMeeting: vi.fn(),
  refetchMeetings: vi.fn(),
  startRecordingWithDevices: vi.fn(),
  clearTranscripts: vi.fn(),
  setMeetingTitle: vi.fn(),
  setIsMeetingActive: vi.fn(),
  setActiveRecordingMeetingId: vi.fn(),
  setStatus: vi.fn(),
}));

vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }));
vi.mock('@/contexts/TranscriptContext', () => ({
  useTranscripts: () => ({ clearTranscripts, setMeetingTitle }),
}));
vi.mock('@/components/Sidebar/SidebarProvider', () => ({
  useSidebar: () => ({ setIsMeetingActive, setCurrentMeeting, setActiveRecordingMeetingId, refetchMeetings }),
}));
vi.mock('@/contexts/ConfigContext', () => ({
  useConfig: () => ({ selectedDevices: { micDevice: null, systemDevice: null } }),
}));
vi.mock('@/contexts/RecordingStateContext', () => ({
  useRecordingState: () => ({ setStatus }),
  RecordingStatus: { IDLE: 'idle', STARTING: 'starting', RECORDING: 'recording', ERROR: 'error' },
}));
vi.mock('@/services/recordingService', () => ({
  recordingService: { startRecordingWithDevices },
}));
vi.mock('@/lib/recordingNotification', () => ({ showRecordingNotification: vi.fn() }));
vi.mock('sonner', () => ({
  toast: Object.assign(vi.fn(), {
    info: vi.fn(),
    error: vi.fn(),
    success: vi.fn(),
    warning: vi.fn(),
    dismiss: vi.fn(),
  }),
}));

import { useRecordingStart } from '@/hooks/useRecordingStart';
import { joinAndRecord } from '@/lib/calendar';

const PENDING_JOIN_MEETING_KEY = 'nixon-join-and-record-meeting';

type Deferred = { resolve: (v: unknown) => void; reject: (e: unknown) => void };

/**
 * Route invoke. Calendar-linked creates (calendarEventId present) return
 * 'meeting-linked' unless deferred; plain creates return 'meeting-fresh'.
 */
function routeInvoke({ deferCalendarCreate }: { deferCalendarCreate?: Deferred[] } = {}) {
  invokeMock.mockImplementation((cmd: string, args?: Record<string, unknown>) => {
    switch (cmd) {
      case 'parakeet_init':
        return Promise.resolve();
      case 'parakeet_has_available_models':
        return Promise.resolve(true);
      case 'api_create_meeting':
        if (args?.calendarEventId) {
          if (deferCalendarCreate) {
            return new Promise((resolve, reject) => {
              deferCalendarCreate.push({ resolve, reject });
            });
          }
          return Promise.resolve({ meeting_id: 'meeting-linked' });
        }
        return Promise.resolve({ meeting_id: 'meeting-fresh' });
      default:
        return Promise.resolve(undefined);
    }
  });
}

function createMeetingCalls() {
  return invokeMock.mock.calls.filter((c) => c[0] === 'api_create_meeting');
}

describe('useRecordingStart — pending-join adoption (specs/0029 WS2.1)', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    sessionStorage.clear();
  });

  it('awaits the in-flight Join & Record create instead of minting its own row', async () => {
    const deferred: Deferred[] = [];
    routeInvoke({ deferCalendarCreate: deferred });

    // Join & Record arms the stash and starts the pre-create (deferred). The user
    // starts recording (zoom-detected toast / tray) BEFORE the create resolves.
    void joinAndRecord(
      { id: 'evt-1', title: 'Weekly Sync', startsAt: '2026-07-01T17:00:00Z' },
      false,
      vi.fn(),
    );

    const { result } = renderHook(() => useRecordingStart(false, vi.fn()));
    await act(async () => {
      const started = result.current.handleRecordingStart();
      // The start path is now waiting on the in-flight create; let it land.
      deferred[0].resolve({ meeting_id: 'meeting-cal' });
      await started;
    });

    // Exactly ONE create total (joinAndRecord's own) — the recorder adopted its id
    // rather than falling back to a date-stamped row.
    expect(createMeetingCalls()).toHaveLength(1);
    expect(setCurrentMeeting).toHaveBeenCalledWith({ id: 'meeting-cal', title: 'Weekly Sync' });
    expect(setActiveRecordingMeetingId).toHaveBeenCalledWith('meeting-cal');
    expect(setMeetingTitle).toHaveBeenCalledWith('Weekly Sync');
    // Attendee seeding fires for the adopted row.
    expect(invokeMock).toHaveBeenCalledWith('api_get_meeting_participants', {
      meetingId: 'meeting-cal',
    });
  });

  it('adopts the pending calendar row in the early-return path instead of discarding it', async () => {
    routeInvoke();
    const { result } = renderHook(() => useRecordingStart(false, vi.fn()));

    // A plain start wins the race first: date-stamped row 'meeting-fresh'.
    await act(async () => {
      await result.current.handleRecordingStart();
    });
    expect(setActiveRecordingMeetingId).toHaveBeenLastCalledWith('meeting-fresh');

    // A pending join (pre-created row) arrives while the session guard is claimed.
    sessionStorage.setItem(
      PENDING_JOIN_MEETING_KEY,
      JSON.stringify({ id: 'meeting-cal', title: 'Weekly Sync', calendarEventId: 'evt-1' }),
    );
    await act(async () => {
      await result.current.handleRecordingStart();
    });

    // No extra create — the pre-created calendar row is adopted as the session row.
    expect(createMeetingCalls()).toHaveLength(1);
    expect(setActiveRecordingMeetingId).toHaveBeenLastCalledWith('meeting-cal');
    expect(setCurrentMeeting).toHaveBeenLastCalledWith({ id: 'meeting-cal', title: 'Weekly Sync' });
    expect(setMeetingTitle).toHaveBeenLastCalledWith('Weekly Sync');
    expect(invokeMock).toHaveBeenCalledWith('api_get_meeting_participants', {
      meetingId: 'meeting-cal',
    });
    // The pending key is consumed — it can't leak into the next recording.
    expect(sessionStorage.getItem(PENDING_JOIN_MEETING_KEY)).toBeNull();
  });

  it('early-return with an id-less pending join creates ONE calendar-linked row and adopts it', async () => {
    routeInvoke();
    const { result } = renderHook(() => useRecordingStart(false, vi.fn()));

    await act(async () => {
      await result.current.handleRecordingStart();
    });

    // Pre-create failed (id null) but the calendar link survived.
    sessionStorage.setItem(
      PENDING_JOIN_MEETING_KEY,
      JSON.stringify({
        id: null,
        title: 'Weekly Sync',
        calendarEventId: 'evt-1',
        startsAt: '2026-07-01T17:00:00Z',
      }),
    );
    await act(async () => {
      await result.current.handleRecordingStart();
    });

    // One plain create (the racing row) + one calendar-LINKED create for adoption,
    // carrying calendarEventId so the backend dedupe can engage.
    expect(invokeMock).toHaveBeenCalledWith('api_create_meeting', {
      meetingTitle: 'Weekly Sync',
      origin: 'recorded',
      calendarEventId: 'evt-1',
      startedAt: '2026-07-01T17:00:00Z',
      calendarSeriesKey: null,
    });
    expect(setActiveRecordingMeetingId).toHaveBeenLastCalledWith('meeting-linked');
    expect(setCurrentMeeting).toHaveBeenLastCalledWith({
      id: 'meeting-linked',
      title: 'Weekly Sync',
    });
  });

  it('re-titles the racing row when the pending join carries no resolvable calendar row', async () => {
    routeInvoke();
    const { result } = renderHook(() => useRecordingStart(false, vi.fn()));

    await act(async () => {
      await result.current.handleRecordingStart();
    });

    // Degenerate stash: title only (no id, no calendar link) — the event title must
    // still stick to the session's row instead of being dropped.
    sessionStorage.setItem(
      PENDING_JOIN_MEETING_KEY,
      JSON.stringify({ id: null, title: 'Weekly Sync' }),
    );
    await act(async () => {
      await result.current.handleRecordingStart();
    });

    expect(createMeetingCalls()).toHaveLength(1); // no second row of any kind
    expect(invokeMock).toHaveBeenCalledWith('api_save_meeting_title', {
      meetingId: 'meeting-fresh',
      title: 'Weekly Sync',
    });
    expect(setCurrentMeeting).toHaveBeenLastCalledWith({
      id: 'meeting-fresh',
      title: 'Weekly Sync',
    });
    expect(setMeetingTitle).toHaveBeenLastCalledWith('Weekly Sync');
  });
});
