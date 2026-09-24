import { describe, it, expect, beforeEach, vi } from 'vitest';
import { renderHook, act, waitFor } from '@testing-library/react';

// specs/0075 W4 — reloading /record mid-recording opened "Recover Interrupted Meetings"
// listing the LIVE recording with Delete/Recover: the provider starts at
// isRecording=false until its first get_recording_state reply, and the recovery check ran
// in that gap. These lock (a) recovery waits for the first sync, and (b) the recoverable
// list never contains the live meeting even if the check does run.

const h = vi.hoisted(() => ({
  getRecordingState: vi.fn(),
  invoke: vi.fn(),
  getAllMeetings: vi.fn(),
}));

vi.mock('@/services/recordingService', () => ({
  recordingService: {
    getRecordingState: h.getRecordingState,
    onRecordingStarted: vi.fn().mockResolvedValue(() => {}),
    onRecordingStopped: vi.fn().mockResolvedValue(() => {}),
    onRecordingPaused: vi.fn().mockResolvedValue(() => {}),
    onRecordingResumed: vi.fn().mockResolvedValue(() => {}),
  },
}));
vi.mock('@/lib/safe-listen', () => ({
  makeSafeUnlisten: (fn: () => void) => fn,
  safeListen: () => () => {},
}));
vi.mock('@tauri-apps/api/core', () => ({ invoke: h.invoke }));
vi.mock('@/services/indexedDBService', () => ({
  indexedDBService: {
    getAllMeetings: h.getAllMeetings,
    deleteOldMeetings: vi.fn().mockResolvedValue(undefined),
    deleteSavedMeetings: vi.fn().mockResolvedValue(undefined),
  },
}));
vi.mock('@/services/storageService', () => ({ storageService: {} }));
vi.mock('@/lib/summary-language-preferences', () => ({
  applyPinnedSummaryLanguageToMeeting: vi.fn(),
}));
vi.mock('@/lib/resume-recording', () => ({ isResumeArmedOrInFlight: () => false }));
vi.mock('@/components/Sidebar/SidebarProvider', () => ({
  useSidebar: () => ({ refetchMeetings: vi.fn() }),
}));
vi.mock('next/navigation', () => ({ useRouter: () => ({ push: vi.fn() }) }));
vi.mock('sonner', () => ({ toast: { success: vi.fn(), error: vi.fn(), warning: vi.fn() } }));

import { RecordingStateProvider } from '@/contexts/RecordingStateContext';
import { useStartupRecovery } from '@/hooks/useStartupRecovery';

const wrapper = ({ children }: { children: React.ReactNode }) => (
  <RecordingStateProvider>{children}</RecordingStateProvider>
);

const LIVE_ID = 'meeting-live';
const LIVE_FOLDER = '/rec/Meeting live';
const LIVE_DURATION_S = 120;

function row(meetingId: string, extra: Partial<Record<string, unknown>> = {}) {
  return {
    meetingId,
    title: meetingId,
    startTime: Date.now() - 3 * 3600_000,
    lastUpdated: Date.now() - 60_000,
    transcriptCount: 3,
    savedToSQLite: false,
    ...extra,
  };
}

const liveRow = () =>
  row(LIVE_ID, { folderPath: LIVE_FOLDER, startTime: Date.now() - LIVE_DURATION_S * 1000 });
const interruptedRow = () => row('meeting-old', { folderPath: '/rec/Meeting old' });

function backendState(isRecording: boolean) {
  return {
    is_recording: isRecording,
    is_paused: false,
    is_active: isRecording,
    recording_duration: isRecording ? LIVE_DURATION_S : null,
    active_duration: isRecording ? LIVE_DURATION_S : null,
  };
}

/** Rust-side `invoke` as seen by the recovery filter. */
function backendInvoke(isRecording: boolean) {
  h.invoke.mockImplementation(async (cmd: string) => {
    if (cmd === 'get_recording_state') return backendState(isRecording);
    if (cmd === 'get_meeting_folder_path') return isRecording ? LIVE_FOLDER : null;
    if (cmd === 'has_audio_checkpoints') return true;
    return null;
  });
}

beforeEach(() => {
  vi.clearAllMocks();
  sessionStorage.clear();
});

describe('useStartupRecovery waits for the real recording state (specs/0075 W4)', () => {
  it('does not check or show the dialog while the provider is unsynced', async () => {
    sessionStorage.setItem('indexeddb_current_meeting_id', LIVE_ID);
    h.getRecordingState.mockReturnValue(new Promise(() => {})); // first reply never lands
    backendInvoke(true);
    h.getAllMeetings.mockResolvedValue([liveRow()]);

    const { result } = renderHook(() => useStartupRecovery(), { wrapper });
    await act(async () => {
      await new Promise((r) => setTimeout(r, 30));
    });

    expect(h.getAllMeetings).not.toHaveBeenCalled();
    expect(result.current.showRecoveryDialog).toBe(false);
    expect(result.current.recoverableMeetings).toEqual([]);
  });

  it('still shows nothing once the first poll reports a recording', async () => {
    sessionStorage.setItem('indexeddb_current_meeting_id', LIVE_ID);
    let reply!: (v: unknown) => void;
    h.getRecordingState.mockReturnValue(new Promise((r) => (reply = r)));
    backendInvoke(true);
    h.getAllMeetings.mockResolvedValue([liveRow()]);

    const { result } = renderHook(() => useStartupRecovery(), { wrapper });
    await act(async () => {
      reply(backendState(true));
      await new Promise((r) => setTimeout(r, 30));
    });

    expect(h.getAllMeetings).not.toHaveBeenCalled();
    expect(result.current.showRecoveryDialog).toBe(false);
  });

  it('shows the dialog for a genuinely interrupted meeting when nothing is recording', async () => {
    h.getRecordingState.mockResolvedValue(backendState(false));
    backendInvoke(false);
    h.getAllMeetings.mockResolvedValue([interruptedRow()]);

    const { result } = renderHook(() => useStartupRecovery(), { wrapper });

    await waitFor(() => expect(result.current.showRecoveryDialog).toBe(true));
    expect(result.current.recoverableMeetings.map((m) => m.meetingId)).toEqual(['meeting-old']);
  });

  it('a failed first sync still counts as synced (recovery is not blocked forever)', async () => {
    h.getRecordingState.mockRejectedValue(new Error('ipc down'));
    backendInvoke(false);
    h.getAllMeetings.mockResolvedValue([interruptedRow()]);

    const { result } = renderHook(() => useStartupRecovery(), { wrapper });

    await waitFor(() => expect(result.current.showRecoveryDialog).toBe(true));
  });
});

describe('the recoverable list never includes the live meeting (specs/0075 W4)', () => {
  // The provider's own view says "not recording" (the race the sync gate is meant to
  // close), but the backend is recording: the check runs and must drop the live row.
  const cases: Array<[string, () => void, () => ReturnType<typeof row>]> = [
    [
      'matched by its IndexedDB id (sessionStorage)',
      () => sessionStorage.setItem('indexeddb_current_meeting_id', LIVE_ID),
      () => row(LIVE_ID),
    ],
    ['matched by its recording folder', () => {}, () => row('meeting-x', { folderPath: LIVE_FOLDER })],
    [
      'matched by its start time',
      () => {},
      () => row('meeting-y', { startTime: Date.now() - LIVE_DURATION_S * 1000 - 2000 }),
    ],
  ];

  it.each(cases)('live meeting %s is filtered out', async (_name, arrange, live) => {
    arrange();
    h.getRecordingState.mockResolvedValue(backendState(false));
    backendInvoke(true);
    h.getAllMeetings.mockResolvedValue([live(), interruptedRow()]);

    const { result } = renderHook(() => useStartupRecovery(), { wrapper });

    await waitFor(() => expect(h.getAllMeetings).toHaveBeenCalled());
    await waitFor(() => expect(result.current.showRecoveryDialog).toBe(true));
    expect(result.current.recoverableMeetings.map((m) => m.meetingId)).toEqual(['meeting-old']);
  });
});
