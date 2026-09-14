import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { renderHook, act } from '@testing-library/react';

// specs/0023 L3 — hook-level test for the recording-STOP abandoned-cleanup guard
// (specs/0019 WS6.1). A stop with zero transcripts and no notes is only allowed to
// delete the in-progress meeting when the backend confirms there is nothing durable
// to keep (api_recording_is_safe_to_discard). When the backend says "keep" (calendar-
// linked or on-disk audio), the meeting must be PRESERVED — saved, not deleted.
//
// The handler is large (transcription poll + timed waits), so we use fake timers and
// mock the context/service seam. The poll loop exits immediately because the mocked
// transcription status reports idle.

const {
  invokeMock,
  saveMeeting,
  getMeeting,
  getTranscriptionStatus,
  setCurrentMeeting,
  refetchMeetings,
  setMeetings,
  setIsMeetingActive,
  setActiveRecordingMeetingId,
  clearTranscripts,
  flushBuffer,
  markMeetingAsSaved,
  setStatus,
  routerPush,
  applyPinned,
  detectLang,
  sidebarState,
  enqueueMeetingMock,
  toastWarn,
  toastSuccess,
  toastError,
} = vi.hoisted(() => ({
  invokeMock: vi.fn(),
  saveMeeting: vi.fn(),
  getMeeting: vi.fn(),
  getTranscriptionStatus: vi.fn(),
  setCurrentMeeting: vi.fn(),
  refetchMeetings: vi.fn(),
  setMeetings: vi.fn(),
  setIsMeetingActive: vi.fn(),
  setActiveRecordingMeetingId: vi.fn(),
  clearTranscripts: vi.fn(),
  flushBuffer: vi.fn(),
  markMeetingAsSaved: vi.fn(),
  setStatus: vi.fn(),
  routerPush: vi.fn(),
  applyPinned: vi.fn(),
  detectLang: vi.fn(),
  // spec 0051 WS2: the stop path awaits useBacklog().enqueueMeeting for a 'process-now'
  // handoff. Defaults to accepted; individual tests override the resolved/rejected value.
  enqueueMeetingMock: vi.fn().mockResolvedValue({ accepted: true }),
  // review round 1, finding 2: the previous `new Proxy({}, { get: () => vi.fn() })` sonner
  // mock handed back a FRESH vi.fn() on every property read, so `toast.warning(...)`
  // could never be asserted against — a deleted toast call still passed the full suite.
  // Stable spies fix that.
  toastWarn: vi.fn(),
  toastSuccess: vi.fn(),
  toastError: vi.fn(),
  // Mutable sidebar state so a test can simulate the "current meeting drifted while
  // recording" case (v1.6.1). Reset in beforeEach of the relevant describe.
  sidebarState: {
    currentMeeting: { id: 'meeting-x', title: 'My Meeting' } as { id: string; title: string },
    activeRecordingMeetingId: null as string | null,
  },
}));

// transcriptsRef is a real ref-like object the test controls (empty => abandoned path).
const transcriptsRef = { current: [] as unknown[] };

vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn().mockResolvedValue(() => {}) }));
vi.mock('@/lib/safe-listen', () => ({
  safeListen: vi.fn(() => () => {}),
  makeSafeUnlisten: vi.fn(() => () => {}),
}));
vi.mock('next/navigation', () => ({ useRouter: () => ({ push: routerPush, replace: vi.fn() }) }));
vi.mock('@/contexts/TranscriptContext', () => ({
  useTranscripts: () => ({
    transcriptsRef,
    flushBuffer,
    clearTranscripts,
    meetingTitle: 'My Meeting',
    markMeetingAsSaved,
  }),
}));
vi.mock('@/components/Sidebar/SidebarProvider', () => ({
  useSidebar: () => ({
    refetchMeetings,
    setCurrentMeeting,
    setMeetings,
    meetings: [],
    setIsMeetingActive,
    setActiveRecordingMeetingId,
    currentMeeting: sidebarState.currentMeeting,
    activeRecordingMeetingId: sidebarState.activeRecordingMeetingId,
  }),
}));
vi.mock('@/contexts/RecordingStateContext', () => ({
  useRecordingState: () => ({
    status: 'recording',
    setStatus,
    isStopping: false,
    isProcessing: false,
    isSaving: false,
  }),
  RecordingStatus: {
    IDLE: 'idle',
    STOPPING: 'stopping',
    PROCESSING_TRANSCRIPTS: 'processing',
    SAVING: 'saving',
    COMPLETED: 'completed',
    ERROR: 'error',
  },
}));
vi.mock('@/services/storageService', () => ({ storageService: { saveMeeting, getMeeting } }));
vi.mock('@/services/transcriptService', () => ({ transcriptService: { getTranscriptionStatus } }));
vi.mock('@/lib/summary-language-preferences', () => ({
  applyPinnedSummaryLanguageToMeeting: applyPinned,
  detectAndCacheSummaryLanguage: detectLang,
}));
vi.mock('sonner', () => ({
  toast: { warning: toastWarn, success: toastSuccess, error: toastError },
}));
vi.mock('@/contexts/DeferredBacklogProvider', () => ({
  useBacklog: () => ({ enqueueMeeting: enqueueMeetingMock }),
}));

import { useRecordingStop } from '@/hooks/useRecordingStop';
import { recordStopRecordingResult, clearStopRecordingResult } from '@/lib/recording-stop';

function deleteMeetingCalls() {
  return invokeMock.mock.calls.filter((c) => c[0] === 'api_delete_meeting');
}

/** Drive handleRecordingStop(true) to completion through its timed waits. */
async function runStop(handler: (callApi: boolean) => Promise<void>) {
  await act(async () => {
    const p = handler(true);
    // Push through: 4s late-segment wait + 0.5s state wait + 5s deferred race + 2s nav.
    await vi.advanceTimersByTimeAsync(12000);
    await p;
  });
}

describe('useRecordingStop — abandoned-cleanup guard (WS6.1)', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.useFakeTimers();
    clearStopRecordingResult(); // no in-memory stop result — exercise the event transport
    transcriptsRef.current = []; // zero transcripts => abandoned candidate
    sessionStorage.clear();
    sessionStorage.setItem('last_recording_folder_path', '/recordings/x');
    // Transcription reports idle so the poll loop exits on the first iteration.
    getTranscriptionStatus.mockResolvedValue({
      is_processing: false,
      chunks_in_queue: 0,
      last_activity_ms: 0,
    });
    saveMeeting.mockResolvedValue({ meeting_id: 'meeting-x' });
    getMeeting.mockResolvedValue({ id: 'meeting-x', title: 'My Meeting' });
    applyPinned.mockResolvedValue(true); // skip language detection branch
    invokeMock.mockImplementation((cmd: string) => {
      switch (cmd) {
        case 'api_get_meeting_notes':
          return Promise.resolve(null); // no notes
        case 'api_recording_is_safe_to_discard':
          return Promise.resolve(safeToDiscard);
        default:
          return Promise.resolve(undefined);
      }
    });
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  let safeToDiscard = true;

  it('deletes the empty meeting only when the backend says it is safe to discard', async () => {
    safeToDiscard = true;
    const { result } = renderHook(() => useRecordingStop(vi.fn(), vi.fn()));

    await runStop(result.current.handleRecordingStop);

    expect(invokeMock).toHaveBeenCalledWith('api_recording_is_safe_to_discard', {
      meetingId: 'meeting-x',
      folderPath: '/recordings/x',
    });
    expect(deleteMeetingCalls()).toHaveLength(1);
    expect(invokeMock).toHaveBeenCalledWith('api_delete_meeting', { meetingId: 'meeting-x' });
    // Discarded, so it must NOT have been saved.
    expect(saveMeeting).not.toHaveBeenCalled();
  });

  it('preserves the meeting (saves, does not delete) when the backend says keep it', async () => {
    safeToDiscard = false;
    const { result } = renderHook(() => useRecordingStop(vi.fn(), vi.fn()));

    await runStop(result.current.handleRecordingStop);

    // The data-loss fix: a calendar-linked / audio-backed meeting is never deleted.
    expect(deleteMeetingCalls()).toHaveLength(0);
    // It falls through to save, attaching to the existing meeting id. specs/0037: a normal
    // (non-resumed) stop saves with resumed=false / audioOffsetSeconds=0.
    expect(saveMeeting).toHaveBeenCalledTimes(1);
    expect(saveMeeting).toHaveBeenCalledWith('My Meeting', [], '/recordings/x', 'meeting-x', false, 0);
    // WS6.5: stopping clears the authoritative live-recording id so a later nav to a
    // past meeting no longer redirects to /record.
    expect(setActiveRecordingMeetingId).toHaveBeenCalledWith(null);
  });
});

// specs/0037 — resumed-session stop routing. The `recording-stopped` payload carries
// `resumed` + `prior_audio_duration_seconds`; the stop handler must forward them to the
// save so a resumed session APPENDS to the same meeting (offset by the prior audio
// duration). A normal stop forwards resumed=false / offset=0. The listener stashes those
// values in sessionStorage (same seam as folder_path), so we seed sessionStorage directly.
describe('useRecordingStop — resumed append routing (specs/0037)', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.useFakeTimers();
    clearStopRecordingResult(); // no in-memory stop result — exercise the event transport
    transcriptsRef.current = [{ text: 'part' }]; // transcripts present => save path (not abandoned)
    sessionStorage.clear();
    sessionStorage.setItem('last_recording_folder_path', '/recordings/x');
    getTranscriptionStatus.mockResolvedValue({
      is_processing: false,
      chunks_in_queue: 0,
      last_activity_ms: 0,
    });
    saveMeeting.mockResolvedValue({ meeting_id: 'meeting-x' });
    getMeeting.mockResolvedValue({ id: 'meeting-x', title: 'My Meeting' });
    applyPinned.mockResolvedValue(true); // skip language detection branch
    invokeMock.mockResolvedValue(undefined);
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it('forwards resumed=true + the prior audio duration as the save offset', async () => {
    sessionStorage.setItem('last_recording_resumed', 'true');
    sessionStorage.setItem('last_recording_prior_audio_duration', '120.5');

    const { result } = renderHook(() => useRecordingStop(vi.fn(), vi.fn()));
    await runStop(result.current.handleRecordingStop);

    expect(saveMeeting).toHaveBeenCalledWith(
      'My Meeting',
      [{ text: 'part' }],
      '/recordings/x',
      'meeting-x',
      true,
      120.5,
    );
  });

  it('forwards resumed=false / offset=0 on a normal (non-resumed) stop', async () => {
    // No resumed stash present — a plain stop.
    const { result } = renderHook(() => useRecordingStop(vi.fn(), vi.fn()));
    await runStop(result.current.handleRecordingStop);

    expect(saveMeeting).toHaveBeenCalledWith(
      'My Meeting',
      [{ text: 'part' }],
      '/recordings/x',
      'meeting-x',
      false,
      0,
    );
  });
});

// v1.6.1 regression — a recording's transcript must attach to the RECORDING's own
// meeting, not the app's "current meeting" selection, which drifts if the user opens
// another meeting while recording (the 0036 Today/prep view made this easy). Before the
// fix, a whole 65-min recording's transcript + summary landed on an unrelated meeting.
describe('useRecordingStop — save target is the recording, not the drifted selection (v1.6.1)', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.useFakeTimers();
    clearStopRecordingResult();
    transcriptsRef.current = [{ text: 'real meeting content' }];
    sessionStorage.clear();
    sessionStorage.setItem('last_recording_folder_path', '/recordings/crm');
    getTranscriptionStatus.mockResolvedValue({ is_processing: false, chunks_in_queue: 0, last_activity_ms: 0 });
    saveMeeting.mockResolvedValue({ meeting_id: 'meeting-recording' });
    getMeeting.mockResolvedValue({ id: 'meeting-recording', title: 'CRM' });
    applyPinned.mockResolvedValue(true);
    invokeMock.mockResolvedValue(undefined);
    // The user drifted the current selection to a DIFFERENT meeting while recording.
    sidebarState.currentMeeting = { id: 'meeting-DRIFTED', title: 'UX Team Sync (prep)' };
    sidebarState.activeRecordingMeetingId = 'meeting-recording';
  });
  afterEach(() => {
    vi.useRealTimers();
    // Restore defaults so this block's drift can't leak into later describes.
    sidebarState.currentMeeting = { id: 'meeting-x', title: 'My Meeting' };
    sidebarState.activeRecordingMeetingId = null;
  });

  it('saves to the backend-reported recording id, overriding the drifted current meeting', async () => {
    recordStopRecordingResult({
      folder_path: '/recordings/crm',
      meeting_name: 'CRM',
      resumed: false,
      prior_audio_duration_seconds: 0,
      meeting_id: 'meeting-recording',
    });
    const { result } = renderHook(() => useRecordingStop(vi.fn(), vi.fn()));
    await runStop(result.current.handleRecordingStop);

    expect(saveMeeting).toHaveBeenCalledTimes(1);
    // 4th arg = existingMeetingId → the recording's id, NOT 'meeting-DRIFTED'.
    expect(saveMeeting.mock.calls[0][3]).toBe('meeting-recording');
  });

  it('falls back to the pinned activeRecordingMeetingId when the stop result carries no id', async () => {
    // Tray/reload path: no in-memory stop result; folder_path comes from the event keys.
    clearStopRecordingResult();
    const { result } = renderHook(() => useRecordingStop(vi.fn(), vi.fn()));
    await runStop(result.current.handleRecordingStop);

    expect(saveMeeting).toHaveBeenCalledTimes(1);
    expect(saveMeeting.mock.calls[0][3]).toBe('meeting-recording');
  });
});

// specs/0037 review-2 (FIX C) — the stop invoke's RETURN VALUE (stashed in memory by
// recordingService.stopRecording) is the PRIMARY transport for resumed / prior-duration /
// folder_path. The `recording-stopped` event fires only after the possibly-slow
// stop_and_save, so on long meetings it can lose the save path's bounded 5s wait — and a
// missed event previously read resumed=false and saved the resumed session into a
// DUPLICATE meeting. The event/sessionStorage keys remain the fallback (tray-initiated
// stops run the command in Rust; a reload mid-stop loses the in-memory stash).
describe('useRecordingStop — stop-invoke return value is the primary transport (FIX C)', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.useFakeTimers();
    clearStopRecordingResult();
    transcriptsRef.current = [{ text: 'part' }];
    sessionStorage.clear();
    getTranscriptionStatus.mockResolvedValue({
      is_processing: false,
      chunks_in_queue: 0,
      last_activity_ms: 0,
    });
    saveMeeting.mockResolvedValue({ meeting_id: 'meeting-x' });
    getMeeting.mockResolvedValue({ id: 'meeting-x', title: 'My Meeting' });
    applyPinned.mockResolvedValue(true);
    invokeMock.mockResolvedValue(undefined);
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it('saves from the invoke result even when the event never arrives and sessionStorage disagrees', async () => {
    // Simulate the exact race: the recording-stopped event is LATE (never delivered here),
    // and whatever sessionStorage holds is stale/default — resumed=false would have saved
    // a duplicate meeting. The invoke result must win on every field.
    sessionStorage.setItem('last_recording_folder_path', '/recordings/stale');
    sessionStorage.setItem('last_recording_meeting_name', 'Stale Name');
    sessionStorage.setItem('last_recording_resumed', 'false');
    sessionStorage.setItem('last_recording_prior_audio_duration', '0');
    recordStopRecordingResult({
      message: 'Recording stopped',
      folder_path: '/recordings/fresh',
      meeting_name: 'Resumed Meeting',
      resumed: true,
      prior_audio_duration_seconds: 321.5,
    });

    const { result } = renderHook(() => useRecordingStop(vi.fn(), vi.fn()));
    await runStop(result.current.handleRecordingStop);

    expect(saveMeeting).toHaveBeenCalledWith(
      'Resumed Meeting',
      [{ text: 'part' }],
      '/recordings/fresh',
      'meeting-x',
      true,
      321.5,
    );
  });

  it('a start-time wipe of the sessionStorage keys cannot misroute an in-flight save', async () => {
    // The invoke result was recorded, then the next start's clearStaleResumeStopKeys wiped
    // the sessionStorage keys before the save read them — the in-memory value must still
    // route the append. (The save consumes it at handler entry, before any await.)
    recordStopRecordingResult({
      folder_path: '/recordings/fresh',
      meeting_name: 'Resumed Meeting',
      resumed: true,
      prior_audio_duration_seconds: 60,
    });
    // sessionStorage intentionally left EMPTY (as after a wipe).

    const { result } = renderHook(() => useRecordingStop(vi.fn(), vi.fn()));
    await runStop(result.current.handleRecordingStop);

    expect(saveMeeting).toHaveBeenCalledWith(
      'Resumed Meeting',
      [{ text: 'part' }],
      '/recordings/fresh',
      'meeting-x',
      true,
      60,
    );
  });

  it('tolerates a backend without the return value (undefined) — falls back to the event keys', async () => {
    recordStopRecordingResult(undefined); // pre-return-contract backend: records nothing
    sessionStorage.setItem('last_recording_folder_path', '/recordings/x');
    sessionStorage.setItem('last_recording_resumed', 'true');
    sessionStorage.setItem('last_recording_prior_audio_duration', '120.5');

    const { result } = renderHook(() => useRecordingStop(vi.fn(), vi.fn()));
    await runStop(result.current.handleRecordingStop);

    expect(saveMeeting).toHaveBeenCalledWith(
      'My Meeting',
      [{ text: 'part' }],
      '/recordings/x',
      'meeting-x',
      true,
      120.5,
    );
  });

  it('is read-and-clear: a later stop cannot reuse a previous stop result', async () => {
    recordStopRecordingResult({
      folder_path: '/recordings/first',
      meeting_name: 'First',
      resumed: true,
      prior_audio_duration_seconds: 10,
    });

    const { result } = renderHook(() => useRecordingStop(vi.fn(), vi.fn()));
    await runStop(result.current.handleRecordingStop);
    expect(saveMeeting).toHaveBeenLastCalledWith('First', [{ text: 'part' }], '/recordings/first', 'meeting-x', true, 10);

    // Second stop: no new invoke result, no event — must NOT reuse the first one.
    sessionStorage.clear();
    sessionStorage.setItem('last_recording_folder_path', '/recordings/second');
    await runStop(result.current.handleRecordingStop);
    expect(saveMeeting).toHaveBeenLastCalledWith(
      'My Meeting',
      [{ text: 'part' }],
      '/recordings/second',
      'meeting-x',
      false,
      0,
    );
  });
});

// spec 0051 WS2 — the stop path writes a durable processing_mode='defer' marker BEFORE
// attempting to hand a 'process-now' meeting to the deferred backlog, and a throwing
// handoff must be caught LOCALLY (converted to {accepted:false, reason:'threw'}) rather
// than left to escape into the generic bookkeeping catch — a null handoff happens to read
// as failure too, so only a message/order assertion (not just the fallback outcome) can
// tell the two implementations apart.
describe('useRecordingStop — 0051 WS2 durable defer marker + acknowledged handoff', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.useFakeTimers();
    clearStopRecordingResult();
    transcriptsRef.current = [{ text: 'part' }];
    sessionStorage.clear();
    sessionStorage.setItem('last_recording_folder_path', '/recordings/x');
    // Session started deferred and is now overridden live => stopAction === 'process-now'.
    sessionStorage.setItem('recording_session_started_deferred', 'true');
    getTranscriptionStatus.mockResolvedValue({
      is_processing: false,
      chunks_in_queue: 0,
      last_activity_ms: 0,
    });
    saveMeeting.mockResolvedValue({ meeting_id: 'meeting-x' });
    getMeeting.mockResolvedValue({ id: 'meeting-x', title: 'My Meeting' });
    applyPinned.mockResolvedValue(true);
    enqueueMeetingMock.mockReset();
    enqueueMeetingMock.mockResolvedValue({ accepted: true });
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'api_get_meeting_processing_mode') return Promise.resolve('live');
      return Promise.resolve(undefined);
    });
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  function deferMarkerCalls() {
    return invokeMock.mock.calls.filter(
      (c) => c[0] === 'api_set_meeting_processing_mode' && c[1]?.mode === 'defer',
    );
  }

  it('writes the durable defer marker BEFORE attempting the process-now handoff', async () => {
    let markerWasSetBeforeHandoff = false;
    enqueueMeetingMock.mockImplementation(async () => {
      markerWasSetBeforeHandoff = deferMarkerCalls().length > 0;
      return { accepted: true };
    });

    const { result } = renderHook(() => useRecordingStop(vi.fn(), vi.fn()));
    await runStop(result.current.handleRecordingStop);

    expect(enqueueMeetingMock).toHaveBeenCalledWith('meeting-x', { force: true });
    expect(markerWasSetBeforeHandoff).toBe(true);
  });

  it('catches a throwing handoff locally instead of letting it escape to the outer bookkeeping catch', async () => {
    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {});
    enqueueMeetingMock.mockRejectedValue(new Error('backlog unavailable'));

    const { result } = renderHook(() => useRecordingStop(vi.fn(), vi.fn()));
    await runStop(result.current.handleRecordingStop);

    // The dedicated catch around the handoff call logs this specific message. If the
    // throw instead escapes to the outer bookkeeping catch, only the generic
    // "Processing-mode bookkeeping failed" warning fires and this one never does.
    expect(warnSpy).toHaveBeenCalledWith('Deferred-backlog handoff threw:', expect.any(Error));
    // The meeting must still complete normally — a refused handoff is recoverable via
    // the durable marker, not a save failure.
    expect(setStatus).toHaveBeenCalledWith('completed');
    expect(setStatus).not.toHaveBeenCalledWith('error', expect.anything());
    warnSpy.mockRestore();
  });

  it('falls back to auto-diarize AND warns the user when the handoff is refused', async () => {
    enqueueMeetingMock.mockResolvedValue({ accepted: false, reason: 'no-folder-path' });
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'api_get_meeting_processing_mode') return Promise.resolve('live');
      if (cmd === 'api_get_diarization_enabled') return Promise.resolve(true);
      if (cmd === 'api_diarization_models_present') return Promise.resolve(true);
      return Promise.resolve(undefined);
    });

    const { result } = renderHook(() => useRecordingStop(vi.fn(), vi.fn()));
    await runStop(result.current.handleRecordingStop);
    // Flush the fire-and-forget auto-diarize microtask chain.
    await act(async () => {
      await vi.advanceTimersByTimeAsync(100);
    });

    expect(invokeMock).toHaveBeenCalledWith('api_diarize_meeting', { meetingId: 'meeting-x' });
    // review round 1, finding 2: "no explanation to the user" is one of the three
    // symptoms this task exists to fix — assert the actual toast call, not just that
    // stopFollowUp() would have returned one.
    expect(toastWarn).toHaveBeenCalledWith(
      "Couldn't start processing this meeting",
      expect.objectContaining({ description: expect.stringContaining('Process now') }),
    );
  });

  // review round 1, finding 3: today's real path to a null handoff with
  // action === 'process-now' is api_set_meeting_processing_mode itself throwing — that
  // jumps straight to the outer bookkeeping catch, `handoff` is never assigned, and
  // enqueueMeeting must never be called (there's no durable marker to make it recoverable
  // via, so attempting the handoff would be pointless). stopFollowUp still treats a null
  // handoff as failure, so the fallback (auto-diarize + warn) must still fire.
  it('never calls enqueueMeeting when writing the defer marker itself throws, and still falls back', async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'api_get_meeting_processing_mode') return Promise.resolve('live');
      if (cmd === 'api_set_meeting_processing_mode') return Promise.reject(new Error('db locked'));
      if (cmd === 'api_get_diarization_enabled') return Promise.resolve(true);
      if (cmd === 'api_diarization_models_present') return Promise.resolve(true);
      return Promise.resolve(undefined);
    });
    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {});

    const { result } = renderHook(() => useRecordingStop(vi.fn(), vi.fn()));
    await runStop(result.current.handleRecordingStop);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(100);
    });

    expect(enqueueMeetingMock).not.toHaveBeenCalled();
    expect(invokeMock).toHaveBeenCalledWith('api_diarize_meeting', { meetingId: 'meeting-x' });
    expect(toastWarn).toHaveBeenCalledWith(
      "Couldn't start processing this meeting",
      expect.objectContaining({ description: expect.stringContaining('Process now') }),
    );
    warnSpy.mockRestore();
  });

  it('marks a still-deferred meeting (mark-defer) without attempting any handoff', async () => {
    invokeMock.mockImplementation((cmd: string) => {
      // No live override recorded => stopAction === 'mark-defer'.
      if (cmd === 'api_get_meeting_processing_mode') return Promise.resolve(null);
      return Promise.resolve(undefined);
    });

    const { result } = renderHook(() => useRecordingStop(vi.fn(), vi.fn()));
    await runStop(result.current.handleRecordingStop);

    expect(deferMarkerCalls().length).toBeGreaterThan(0);
    expect(enqueueMeetingMock).not.toHaveBeenCalled();
  });
});
