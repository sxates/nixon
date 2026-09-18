import { describe, it, expect, beforeEach, vi } from 'vitest';
import { renderHook, act } from '@testing-library/react';

// specs/0061 W2 — the first external user clicked "Identify speakers", got a
// 4-second toast saying a speaker model was downloading, then had no feedback
// at all while ~108 MB downloaded silently. This covers the fix: a persistent
// ("diar-dl") toast that tracks byte progress and only then resolves.
//
// `listeners` maps event name -> the list of handlers currently registered for
// it (one real Tauri event can have many subscribers — e.g. two open meetings
// each mount their own `useDiarization`), so `emit*` below fires every mounted
// hook the way the real event bus would, not just the most recently mounted one.

const { invokeMock, listeners } = vi.hoisted(() => ({
  invokeMock: vi.fn(),
  listeners: new Map<string, Array<(event: { payload: unknown }) => void>>(),
}));
vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }));
vi.mock('@/lib/safe-listen', () => ({
  safeListen: vi.fn(
    (eventName: string, handler: (event: { payload: unknown }) => void) => {
      const handlers = listeners.get(eventName) ?? [];
      handlers.push(handler);
      listeners.set(eventName, handlers);
      return () => {
        const current = listeners.get(eventName);
        if (!current) return;
        listeners.set(
          eventName,
          current.filter((h) => h !== handler),
        );
      };
    },
  ),
}));
vi.mock('sonner', () => ({
  toast: {
    loading: vi.fn(),
    success: vi.fn(),
    error: vi.fn(),
    info: vi.fn(),
  },
}));

import { toast } from 'sonner';
import { useDiarization } from '@/hooks/useDiarization';

const emitDownloadProgress = (payload: {
  stage: string;
  downloaded_bytes: number;
  total_bytes: number;
}) => {
  act(() => {
    for (const handler of listeners.get('diarization-download-progress') ?? []) {
      handler({ payload });
    }
  });
};

// specs/0061 final review (Important 1): the legacy `diarization-progress`
// compat event carries no meeting_id either, and — unlike the byte-progress
// event above — it drives `stage` directly. It must be gated the same way.
const emitCompatProgress = (payload: { stage: string; pct?: number }) => {
  act(() => {
    for (const handler of listeners.get('diarization-progress') ?? []) {
      handler({ payload });
    }
  });
};

describe('useDiarization — model download progress + persistent toast', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    listeners.clear();
  });

  it('shows a persistent loading toast, tracks byte progress, then resolves', async () => {
    let resolveDownload: () => void = () => {};
    const downloadPromise = new Promise<void>((resolve) => {
      resolveDownload = resolve;
    });

    invokeMock.mockImplementation((cmd: string) => {
      switch (cmd) {
        case 'api_diarization_status':
          return Promise.resolve(null);
        case 'api_diarization_models_present':
          return Promise.resolve(false);
        case 'api_download_diarization_models':
          return downloadPromise;
        case 'api_diarize_meeting':
          return Promise.resolve({ started: true, alreadyRunning: false });
        default:
          return Promise.resolve(null);
      }
    });

    const { result } = renderHook(() => useDiarization({ meetingId: 'm1' }));
    await act(async () => {});

    let identifyDone!: Promise<void>;
    act(() => {
      identifyDone = result.current.identifySpeakers();
    });
    // Flush the models-present check so the download invoke fires and the
    // loading toast is raised before we start emitting progress events.
    await act(async () => {});

    expect(toast.loading).toHaveBeenCalledTimes(1);
    expect(toast.loading).toHaveBeenCalledWith(
      'Downloading speaker model',
      expect.objectContaining({ id: 'diar-dl', description: '~108 MB, one time' }),
    );

    emitDownloadProgress({
      stage: 'downloading embedding model · 12.0 MB / 101.0 MB',
      downloaded_bytes: 12_582_912,
      total_bytes: 105_906_176,
    });
    emitDownloadProgress({
      stage: 'downloading embedding model · 50.0 MB / 101.0 MB',
      downloaded_bytes: 52_428_800,
      total_bytes: 105_906_176,
    });

    expect(result.current.downloadProgress?.label).toContain('MB /');

    resolveDownload();
    await act(async () => {
      await identifyDone;
    });

    expect(toast.success).toHaveBeenCalledWith(
      'Speaker model ready',
      expect.objectContaining({ id: 'diar-dl', duration: 3000 }),
    );
  });

  // Important finding 1 (review round 2): the download-progress event carries
  // no meeting_id, so every mounted hook receives it. Only the hook that
  // actually started the download may surface it — otherwise an unrelated,
  // idle meeting's "Identify speakers" button would render the downloading
  // meeting's byte progress, with no per-meeting event ever able to clear it.
  it('does not leak a download it did not start onto a different meeting', async () => {
    invokeMock.mockImplementation((cmd: string) => {
      switch (cmd) {
        case 'api_diarization_status':
          return Promise.resolve(null);
        case 'api_diarization_models_present':
          return Promise.resolve(false);
        case 'api_download_diarization_models':
          return new Promise(() => {}); // never resolves within this test
        case 'api_diarize_meeting':
          return Promise.resolve({ started: true, alreadyRunning: false });
        default:
          return Promise.resolve(null);
      }
    });

    const meetingA = renderHook(() => useDiarization({ meetingId: 'meeting-a' }));
    const meetingB = renderHook(() => useDiarization({ meetingId: 'meeting-b' }));
    await act(async () => {});

    // Meeting A starts "Identify speakers" and kicks off the model download.
    act(() => {
      void meetingA.result.current.identifySpeakers();
    });
    await act(async () => {});

    emitDownloadProgress({
      stage: 'downloading embedding model · 12.0 MB / 101.0 MB',
      downloaded_bytes: 12_582_912,
      total_bytes: 105_906_176,
    });

    expect(meetingA.result.current.downloadProgress?.label).toContain('MB /');
    // Meeting B never touched "Identify speakers" — it must not see A's progress.
    expect(meetingB.result.current.downloadProgress).toBeNull();

    // ...and a later progress tick still must not leak into B.
    emitDownloadProgress({
      stage: 'downloading embedding model · 80.0 MB / 101.0 MB',
      downloaded_bytes: 83_886_080,
      total_bytes: 105_906_176,
    });
    expect(meetingB.result.current.downloadProgress).toBeNull();

    // specs/0061 final review (Important 1): the compat `diarization-progress`
    // event (no meeting_id) still reached every mounted hook and drove `stage`
    // directly, so B's "Identify speakers" button rendered — and, with no
    // per-meeting complete/error event to ever clear it, permanently stuck at
    // — A's download stage text. Only A (running/downloading) may react.
    emitCompatProgress({ stage: 'downloading embedding model · 12.0 MB / 101.0 MB' });
    expect(meetingA.result.current.stage).toBe(
      'downloading embedding model · 12.0 MB / 101.0 MB',
    );
    expect(meetingB.result.current.stage).toBeNull();

    emitCompatProgress({ stage: 'downloading embedding model · 90.0 MB / 101.0 MB' });
    expect(meetingB.result.current.stage).toBeNull();
  });

  // Important finding 2 (review round 2): a download failure must resolve the
  // persistent `diar-dl` toast to an error, not leave it stuck as "loading"
  // forever (sonner loading toasts don't auto-dismiss).
  it('resolves the persistent toast to an error when the download fails', async () => {
    invokeMock.mockImplementation((cmd: string) => {
      switch (cmd) {
        case 'api_diarization_status':
          return Promise.resolve(null);
        case 'api_diarization_models_present':
          return Promise.resolve(false);
        case 'api_download_diarization_models':
          return Promise.reject(new Error('network error'));
        case 'api_diarize_meeting':
          return Promise.resolve({ started: true, alreadyRunning: false });
        default:
          return Promise.resolve(null);
      }
    });

    const { result } = renderHook(() => useDiarization({ meetingId: 'm1' }));
    await act(async () => {});

    await act(async () => {
      await result.current.identifySpeakers();
    });

    expect(toast.loading).toHaveBeenCalledWith(
      'Downloading speaker model',
      expect.objectContaining({ id: 'diar-dl' }),
    );
    // The toast must resolve via the SAME id (not a second, unrelated toast),
    // and must never have been resolved with toast.success.
    expect(toast.error).toHaveBeenCalledWith(
      'Could not download speaker model',
      expect.objectContaining({ id: 'diar-dl', description: 'network error' }),
    );
    expect(toast.success).not.toHaveBeenCalled();
    expect(result.current.downloadProgress).toBeNull();
  });
});
