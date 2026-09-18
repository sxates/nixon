import { describe, it, expect, beforeEach, vi } from 'vitest';
import { renderHook, act, waitFor } from '@testing-library/react';

// specs/0037 — resume/continue a recording. When a resume is armed (the `resumeRecording`
// sessionStorage stash, set by the relaunch prompt or the meeting-details "Continue
// recording" action), the /record page's useRecordingStart must:
//   - SKIP api_create_meeting entirely (no second meeting row for one conversation);
//   - adopt the EXISTING meeting_id as the session's authoritative recording id;
//   - thread { meetingId, resumeFolderPath } into the start invoke so the backend appends.
// A normal (no-stash) start must stay byte-identical to today (single-key start payload,
// exactly one fresh meeting row).
//
// Review-2 hardening covered below:
//   FIX A — the failed-start orphan delete is scoped to the row THIS attempt inserted,
//           disarmed the moment the backend start resolves, never fires on an
//           "already in progress" failure, and runs the shared discard safety gate.
//   FIX B — a null stash folderPath is RESOLVED from the meeting row (never threaded as
//           null into the start invoke — the backend would treat that as a fresh,
//           non-resume recording and the stop would save a DUPLICATE meeting); a meeting
//           with no folder ABORTS the resume with a user-facing toast.
//   FIX D — after a successful resume, the crashed session's IndexedDB recovery entries
//           (matched by folder — they're keyed by fabricated ids) are marked saved.
//
// Mock seams mirror useRecordingStart.test.ts.

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
  getAllMeetings,
  markMeetingSaved,
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
  getAllMeetings: vi.fn(),
  markMeetingSaved: vi.fn(),
}));

vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }));
vi.mock('@/contexts/TranscriptContext', () => ({
  useTranscripts: () => ({ clearTranscripts, setMeetingTitle }),
}));
vi.mock('@/components/Sidebar/SidebarProvider', () => ({
  useSidebar: () => ({
    setIsMeetingActive,
    setCurrentMeeting,
    setActiveRecordingMeetingId,
    activeRecordingMeetingId: null,
    refetchMeetings,
  }),
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
vi.mock('@/services/indexedDBService', () => ({
  indexedDBService: { getAllMeetings, markMeetingSaved },
}));
vi.mock('@/lib/recordingNotification', () => ({ showRecordingNotification: vi.fn() }));
vi.mock('sonner', () => ({
  toast: { info: vi.fn(), error: vi.fn(), success: vi.fn(), warning: vi.fn() },
}));

import { toast } from 'sonner';
import { useRecordingStart } from '@/hooks/useRecordingStart';
import { armResumeRecording, RESUME_RECORDING_KEY } from '@/lib/resume-recording';

function routeInvoke({
  parakeetReady = true,
  metadataFolderPath = '/recordings/from-row' as string | null,
  meetingNotes = null as { notesMarkdown: string | null; notesJson: string | null } | null,
  safeToDiscard = true,
}: {
  parakeetReady?: boolean;
  metadataFolderPath?: string | null;
  meetingNotes?: { notesMarkdown: string | null; notesJson: string | null } | null;
  safeToDiscard?: boolean;
} = {}) {
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
      case 'api_get_meeting_metadata':
        return Promise.resolve(
          metadataFolderPath ? { id: 'meeting-existing', folder_path: metadataFolderPath } : { id: 'meeting-existing' },
        );
      case 'api_get_meeting_notes':
        return Promise.resolve(meetingNotes);
      case 'api_recording_is_safe_to_discard':
        return Promise.resolve(safeToDiscard);
      default:
        return Promise.resolve(undefined);
    }
  });
}

function createMeetingCalls() {
  return invokeMock.mock.calls.filter((c) => c[0] === 'api_create_meeting');
}

function deleteMeetingCalls() {
  return invokeMock.mock.calls.filter((c) => c[0] === 'api_delete_meeting');
}

describe('useRecordingStart — resume mode (specs/0037)', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    sessionStorage.clear();
    routeInvoke();
    getAllMeetings.mockResolvedValue([]);
    markMeetingSaved.mockResolvedValue(undefined);
  });

  it('skips api_create_meeting and adopts the existing id, threading resumeFolderPath through', async () => {
    sessionStorage.setItem(
      RESUME_RECORDING_KEY,
      JSON.stringify({
        meetingId: 'meeting-existing',
        folderPath: '/recordings/prev',
        meetingName: 'Prev Meeting',
      }),
    );

    renderHook(() => useRecordingStart(false, vi.fn()));

    // The resume runs from a mount effect (mirror of autoStartRecording); let it settle.
    await waitFor(() => expect(startRecordingWithDevices).toHaveBeenCalledTimes(1));

    // No new meeting row minted — the existing one is reused.
    expect(createMeetingCalls()).toHaveLength(0);

    // Start invoke carries the resume args so the backend appends to the same meeting.
    expect(startRecordingWithDevices).toHaveBeenCalledWith(null, null, 'Prev Meeting', {
      meetingId: 'meeting-existing',
      resumeFolderPath: '/recordings/prev',
    });

    // The existing id becomes the session's authoritative recording id.
    expect(setActiveRecordingMeetingId).toHaveBeenCalledWith('meeting-existing');
    expect(setCurrentMeeting).toHaveBeenCalledWith({ id: 'meeting-existing', title: 'Prev Meeting' });
    expect(setMeetingTitle).toHaveBeenCalledWith('Prev Meeting');

    // The stash is consumed so a resume fires exactly once.
    expect(sessionStorage.getItem(RESUME_RECORDING_KEY)).toBeNull();
  });

  it('resolves a null folderPath from the meeting row and threads THAT folder (FIX B)', async () => {
    routeInvoke({ metadataFolderPath: '/recordings/resolved' });
    sessionStorage.setItem(
      RESUME_RECORDING_KEY,
      JSON.stringify({ meetingId: 'meeting-existing', folderPath: null, meetingName: 'Prev' }),
    );

    renderHook(() => useRecordingStart(false, vi.fn()));
    await waitFor(() => expect(startRecordingWithDevices).toHaveBeenCalledTimes(1));

    // The folder came from the meeting row — NEVER a null resumeFolderPath (the backend
    // treats that as a fresh non-resume start, which saves a duplicate meeting at stop).
    expect(invokeMock).toHaveBeenCalledWith('api_get_meeting_metadata', {
      meetingId: 'meeting-existing',
    });
    expect(startRecordingWithDevices).toHaveBeenCalledWith(null, null, 'Prev', {
      meetingId: 'meeting-existing',
      resumeFolderPath: '/recordings/resolved',
    });
    expect(createMeetingCalls()).toHaveLength(0);
  });

  it('ABORTS the resume with a toast when the meeting has no recording folder (FIX B)', async () => {
    routeInvoke({ metadataFolderPath: null });
    sessionStorage.setItem(
      RESUME_RECORDING_KEY,
      JSON.stringify({ meetingId: 'meeting-existing', folderPath: null, meetingName: 'Prev' }),
    );

    renderHook(() => useRecordingStart(false, vi.fn()));

    await waitFor(() => expect(toast.error).toHaveBeenCalled());
    expect(toast.error).toHaveBeenCalledWith(
      "Can't continue this recording",
      expect.objectContaining({ description: expect.stringContaining('recording folder is missing') }),
    );

    // No start, no new row, no adoption — a resume never degrades into a fresh recording.
    expect(startRecordingWithDevices).not.toHaveBeenCalled();
    expect(createMeetingCalls()).toHaveLength(0);
    expect(setActiveRecordingMeetingId).not.toHaveBeenCalled();
    expect(setStatus).toHaveBeenLastCalledWith('idle');
  });

  it('a normal (no-resume) start is unchanged — one fresh row, no resume args', async () => {
    const { result } = renderHook(() => useRecordingStart(false, vi.fn()));

    // No stash was armed, so the mount effect must not resume anything.
    expect(startRecordingWithDevices).not.toHaveBeenCalled();

    await act(async () => {
      await result.current.handleRecordingStart();
    });

    expect(createMeetingCalls()).toHaveLength(1);
    // GAP 1 (specs/0037): a normal start now threads the freshly-created meeting id so the
    // backend can write it into metadata.json — but NO resumeFolderPath (that's resume-only).
    expect(startRecordingWithDevices).toHaveBeenCalledTimes(1);
    expect(startRecordingWithDevices).toHaveBeenCalledWith(null, null, expect.any(String), {
      meetingId: 'meeting-fresh',
    });
  });
});

describe('useRecordingStart — orphan cleanup on failed start (specs/0037 FIX #6 + review-2 FIX A)', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    sessionStorage.clear();
    routeInvoke();
    getAllMeetings.mockResolvedValue([]);
    markMeetingSaved.mockResolvedValue(undefined);
  });

  it('deletes the just-created row when the backend start throws on a fresh start', async () => {
    startRecordingWithDevices.mockRejectedValueOnce(new Error('audio-capture permission denied'));

    const { result } = renderHook(() => useRecordingStart(false, vi.fn()));

    await act(async () => {
      // handleRecordingStart re-throws so RecordingControls can surface device errors.
      await expect(result.current.handleRecordingStart()).rejects.toThrow();
    });

    // The delete is gated on the SAME safety checks as the stop path's abandoned cleanup:
    // no typed notes + the backend confirming nothing durable exists.
    expect(invokeMock).toHaveBeenCalledWith('api_get_meeting_notes', { meetingId: 'meeting-fresh' });
    expect(invokeMock).toHaveBeenCalledWith('api_recording_is_safe_to_discard', {
      meetingId: 'meeting-fresh',
      folderPath: null,
    });

    // The row created BEFORE the failed start is removed, not stranded.
    expect(createMeetingCalls()).toHaveLength(1);
    expect(deleteMeetingCalls()).toHaveLength(1);
    expect(invokeMock).toHaveBeenCalledWith('api_delete_meeting', { meetingId: 'meeting-fresh' });

    // Adopted-session state is reset back to the placeholder / no active recording.
    expect(setActiveRecordingMeetingId).toHaveBeenLastCalledWith(null);
    expect(setCurrentMeeting).toHaveBeenLastCalledWith({ id: 'intro-call', title: '+ New Call' });
  });

  it('a second start attempt failing "already in progress" never deletes the LIVE meeting (FIX A)', async () => {
    const { result } = renderHook(() => useRecordingStart(false, vi.fn()));

    // First start succeeds: creates meeting-fresh and goes live. A successful backend
    // start DISARMS the orphan cleanup immediately (not via the isRecording effect).
    await act(async () => {
      await result.current.handleRecordingStart();
    });
    expect(createMeetingCalls()).toHaveLength(1);
    expect(deleteMeetingCalls()).toHaveLength(0);

    // Second attempt (double-click / stale isRecording): the create early-returns and the
    // backend rejects. Previously the still-armed ref deleted the ACTIVE session's row.
    startRecordingWithDevices.mockRejectedValueOnce(new Error('Recording already in progress'));
    await act(async () => {
      await expect(result.current.handleRecordingStart()).rejects.toThrow();
    });

    expect(deleteMeetingCalls()).toHaveLength(0);
    // The live session's pointers were never reset to the placeholder.
    expect(setCurrentMeeting).not.toHaveBeenCalledWith({ id: 'intro-call', title: '+ New Call' });
    expect(setActiveRecordingMeetingId).not.toHaveBeenCalledWith(null);
  });

  it('never deletes a meeting with typed notes, even when this attempt created it (FIX A gate)', async () => {
    routeInvoke({ meetingNotes: { notesMarkdown: 'do not lose me', notesJson: null } });
    startRecordingWithDevices.mockRejectedValueOnce(new Error('device gone'));

    const { result } = renderHook(() => useRecordingStart(false, vi.fn()));
    await act(async () => {
      await expect(result.current.handleRecordingStart()).rejects.toThrow();
    });

    expect(createMeetingCalls()).toHaveLength(1);
    expect(deleteMeetingCalls()).toHaveLength(0);
  });

  it('keeps the row when the backend says it is not safe to discard (FIX A gate)', async () => {
    routeInvoke({ safeToDiscard: false });
    startRecordingWithDevices.mockRejectedValueOnce(new Error('device gone'));

    const { result } = renderHook(() => useRecordingStart(false, vi.fn()));
    await act(async () => {
      await expect(result.current.handleRecordingStart()).rejects.toThrow();
    });

    expect(deleteMeetingCalls()).toHaveLength(0);
  });

  it('does NOT delete the existing meeting when a RESUME start throws', async () => {
    sessionStorage.setItem(
      RESUME_RECORDING_KEY,
      JSON.stringify({ meetingId: 'meeting-existing', folderPath: '/recordings/prev', meetingName: 'Prev' }),
    );
    startRecordingWithDevices.mockRejectedValueOnce(new Error('device gone'));

    renderHook(() => useRecordingStart(false, vi.fn()));

    // The resume runs from the mount effect; wait for the (failing) start to be attempted.
    await waitFor(() => expect(startRecordingWithDevices).toHaveBeenCalledTimes(1));
    // Give the rejected promise's catch a tick to run.
    await act(async () => { await Promise.resolve(); });

    // A resumed/adopted existing meeting must never be deleted on failure.
    expect(deleteMeetingCalls()).toHaveLength(0);
    // And no fresh row was ever created for a resume.
    expect(createMeetingCalls()).toHaveLength(0);
  });
});

describe('useRecordingStart — stale resumed keys cleared at start (specs/0037 FIX #7)', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    sessionStorage.clear();
    routeInvoke();
    getAllMeetings.mockResolvedValue([]);
  });

  it('clears last_recording_resumed + prior_audio_duration when a fresh start begins', async () => {
    // Stale keys left by a prior resumed session whose save errored / was navigated away from.
    sessionStorage.setItem('last_recording_resumed', 'true');
    sessionStorage.setItem('last_recording_prior_audio_duration', '123.4');

    const { result } = renderHook(() => useRecordingStart(false, vi.fn()));
    await act(async () => {
      await result.current.handleRecordingStart();
    });

    // Both keys are gone, so the NEXT stop can't be misrouted into the append path.
    expect(sessionStorage.getItem('last_recording_resumed')).toBeNull();
    expect(sessionStorage.getItem('last_recording_prior_audio_duration')).toBeNull();
  });
});

describe('useRecordingStart — arm-while-mounted resume (specs/0037 FIX #8)', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    sessionStorage.clear();
    routeInvoke();
    getAllMeetings.mockResolvedValue([]);
    markMeetingSaved.mockResolvedValue(undefined);
  });

  it('starts the resume when armed while /record is already mounted (event, no remount)', async () => {
    // Mount with nothing armed — the mount effect is a no-op (mirrors idling on /record).
    renderHook(() => useRecordingStart(false, vi.fn()));
    expect(startRecordingWithDevices).not.toHaveBeenCalled();

    // Arm a resume the way the meeting-details / relaunch-prompt actions do. Since the hook
    // is already mounted, the mount effect won't re-run — the `nixon:resume-armed` event must
    // drive the resume instead.
    await act(async () => {
      armResumeRecording({
        meetingId: 'meeting-existing',
        folderPath: '/recordings/prev',
        meetingName: 'Prev Meeting',
      });
    });

    await waitFor(() => expect(startRecordingWithDevices).toHaveBeenCalledTimes(1));
    expect(startRecordingWithDevices).toHaveBeenCalledWith(null, null, 'Prev Meeting', {
      meetingId: 'meeting-existing',
      resumeFolderPath: '/recordings/prev',
    });
    expect(createMeetingCalls()).toHaveLength(0);
    // Consumed exactly once — the stash is cleared, so no double-start.
    expect(sessionStorage.getItem(RESUME_RECORDING_KEY)).toBeNull();
  });
});

describe('useRecordingStart — resumed meeting recovery-entry reconciliation (review-2 FIX D)', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    sessionStorage.clear();
    routeInvoke();
    markMeetingSaved.mockResolvedValue(undefined);
  });

  it('marks the crashed session IndexedDB entries for the resumed folder as saved', async () => {
    // Entries are keyed by TranscriptContext's fabricated `meeting-<ts>` ids, so the
    // match is by recording folder. Only the resumed folder's entries are touched.
    getAllMeetings.mockResolvedValue([
      { meetingId: 'meeting-1719', folderPath: '/recordings/prev', savedToSQLite: false },
      { meetingId: 'meeting-1720', folderPath: '/recordings/other', savedToSQLite: false },
      { meetingId: 'meeting-1721', savedToSQLite: false }, // no folder — never matched
    ]);
    sessionStorage.setItem(
      RESUME_RECORDING_KEY,
      JSON.stringify({ meetingId: 'meeting-existing', folderPath: '/recordings/prev', meetingName: 'Prev' }),
    );

    renderHook(() => useRecordingStart(false, vi.fn()));
    await waitFor(() => expect(startRecordingWithDevices).toHaveBeenCalledTimes(1));

    await waitFor(() => expect(markMeetingSaved).toHaveBeenCalledWith('meeting-1719'));
    expect(markMeetingSaved).toHaveBeenCalledTimes(1);
    expect(markMeetingSaved).not.toHaveBeenCalledWith('meeting-1720');
  });
});
