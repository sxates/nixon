import { describe, it, expect, beforeEach, vi } from 'vitest';
import { renderHook, act } from '@testing-library/react';

// specs/0061 W2 — the first external user clicked "Identify speakers", got a
// 4-second toast saying a speaker model was downloading, then had no feedback
// at all while ~108 MB downloaded silently. This covers the fix: a persistent
// ("diar-dl") toast that tracks byte progress and only then resolves.

const { invokeMock, listeners } = vi.hoisted(() => ({
  invokeMock: vi.fn(),
  listeners: new Map<string, (event: { payload: unknown }) => void>(),
}));
vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }));
vi.mock('@/lib/safe-listen', () => ({
  safeListen: vi.fn(
    (eventName: string, handler: (event: { payload: unknown }) => void) => {
      listeners.set(eventName, handler);
      return () => listeners.delete(eventName);
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
    listeners.get('diarization-download-progress')?.({ payload });
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
});
