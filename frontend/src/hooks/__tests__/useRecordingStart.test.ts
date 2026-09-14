import { describe, it, expect, beforeEach, vi } from 'vitest';
import { renderHook, act } from '@testing-library/react';

// specs/0023 L3 — hook-level lifecycle test for the recording-START meeting-id
// contract that specs/0019 WS6.7 hinges on:
//   - a normal start mints exactly ONE fresh meeting row and adopts its id;
//   - Join & Record adopts the pre-created calendar row (no second INSERT) and
//     consumes its pending-key so it can't leak into the next recording;
//   - the per-session guard prevents a second create within one recording.
//
// The hook is coupled to four contexts + services; we mock those at the module
// boundary (the seam the spec recommends) and drive the real `handleRecordingStart`.
// lib/calendar is intentionally NOT mocked so the real sessionStorage-backed
// consumePendingJoinMeeting runs.

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
  toast: { info: vi.fn(), error: vi.fn(), success: vi.fn(), warning: vi.fn() },
}));

import { useRecordingStart } from '@/hooks/useRecordingStart';

const PENDING_JOIN_MEETING_KEY = 'nixon-join-and-record-meeting';

function routeInvoke({ parakeetReady = true }: { parakeetReady?: boolean } = {}) {
  invokeMock.mockImplementation((cmd: string) => {
    switch (cmd) {
      case 'parakeet_init':
        return Promise.resolve();
      case 'parakeet_has_available_models':
        return Promise.resolve(parakeetReady);
      case 'parakeet_get_available_models':
        return Promise.resolve([]);
      case 'api_create_meeting':
        return Promise.resolve({ meeting_id: 'meeting-fresh' });
      default:
        return Promise.resolve(undefined);
    }
  });
}

function createMeetingCalls() {
  return invokeMock.mock.calls.filter((c) => c[0] === 'api_create_meeting');
}

describe('useRecordingStart — meeting-id lifecycle (WS6.7)', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    sessionStorage.clear();
    routeInvoke();
  });

  it('mints exactly one fresh meeting row and adopts its id on a normal start', async () => {
    const { result } = renderHook(() => useRecordingStart(false, vi.fn()));

    await act(async () => {
      await result.current.handleRecordingStart();
    });

    expect(createMeetingCalls()).toHaveLength(1);
    expect(invokeMock).toHaveBeenCalledWith('api_create_meeting', {
      meetingTitle: expect.any(String),
      folderPath: null,
    });
    expect(setCurrentMeeting).toHaveBeenCalledWith({
      id: 'meeting-fresh',
      title: expect.any(String),
    });
    expect(startRecordingWithDevices).toHaveBeenCalledTimes(1);
    // GAP 1 (specs/0037): the created id is threaded into the start invoke (no resumeFolderPath).
    expect(startRecordingWithDevices).toHaveBeenCalledWith(null, null, expect.any(String), {
      meetingId: 'meeting-fresh',
    });
  });

  it('adopts a Join & Record pre-created row without a second INSERT, and consumes the pending key', async () => {
    sessionStorage.setItem(
      PENDING_JOIN_MEETING_KEY,
      JSON.stringify({ id: 'meeting-calendar', title: 'Weekly Sync' }),
    );

    const { result } = renderHook(() => useRecordingStart(false, vi.fn()));
    await act(async () => {
      await result.current.handleRecordingStart();
    });

    // No new meeting created — the calendar-linked row is adopted as-is.
    expect(createMeetingCalls()).toHaveLength(0);
    expect(setCurrentMeeting).toHaveBeenCalledWith({
      id: 'meeting-calendar',
      title: 'Weekly Sync',
    });
    // Pending key consumed so it can't leak into the next recording.
    expect(sessionStorage.getItem(PENDING_JOIN_MEETING_KEY)).toBeNull();
  });

  it('creates a calendar-LINKED row (and seeds the roster) when pre-create failed (WS6.3 degrade path)', async () => {
    // id null + a surviving calendar link → the recorder creates the row itself with
    // origin 'recorded' + calendarEventId so the live recording keeps the event identity.
    sessionStorage.setItem(
      PENDING_JOIN_MEETING_KEY,
      JSON.stringify({
        id: null,
        title: 'Weekly Sync',
        calendarEventId: 'evt-1',
        startsAt: '2026-06-29T17:00:00Z',
      }),
    );

    const { result } = renderHook(() => useRecordingStart(false, vi.fn()));
    await act(async () => {
      await result.current.handleRecordingStart();
    });

    expect(invokeMock).toHaveBeenCalledWith('api_create_meeting', {
      meetingTitle: 'Weekly Sync',
      origin: 'recorded',
      calendarEventId: 'evt-1',
      startedAt: '2026-06-29T17:00:00Z',
      calendarSeriesKey: null,
    });
    expect(setCurrentMeeting).toHaveBeenCalledWith({
      id: 'meeting-fresh',
      title: 'Weekly Sync',
    });
    // Roster seeded so the live recording shows attendees immediately.
    expect(invokeMock).toHaveBeenCalledWith('api_get_meeting_participants', {
      meetingId: 'meeting-fresh',
    });
  });

  it('seeds the roster when adopting a pre-created calendar-linked row', async () => {
    sessionStorage.setItem(
      PENDING_JOIN_MEETING_KEY,
      JSON.stringify({ id: 'meeting-calendar', title: 'Weekly Sync', calendarEventId: 'evt-2' }),
    );
    const { result } = renderHook(() => useRecordingStart(false, vi.fn()));
    await act(async () => {
      await result.current.handleRecordingStart();
    });

    expect(createMeetingCalls()).toHaveLength(0); // adopted, not re-created
    expect(invokeMock).toHaveBeenCalledWith('api_get_meeting_participants', {
      meetingId: 'meeting-calendar',
    });
  });

  it('does not create a second meeting when start is invoked twice in one session', async () => {
    // isRecording stays false (setIsRecording is mocked), so the per-session guard
    // ref is not reset between calls — the second start must not mint another row.
    const { result } = renderHook(() => useRecordingStart(false, vi.fn()));

    await act(async () => {
      await result.current.handleRecordingStart();
    });
    await act(async () => {
      await result.current.handleRecordingStart();
    });

    expect(createMeetingCalls()).toHaveLength(1);
  });

  it('blocks recording (and creates nothing) when the transcription model is not ready', async () => {
    routeInvoke({ parakeetReady: false });
    const { result } = renderHook(() => useRecordingStart(false, vi.fn()));

    await act(async () => {
      await result.current.handleRecordingStart();
    });

    expect(startRecordingWithDevices).not.toHaveBeenCalled();
    expect(createMeetingCalls()).toHaveLength(0);
    expect(setCurrentMeeting).not.toHaveBeenCalled();
  });
});
