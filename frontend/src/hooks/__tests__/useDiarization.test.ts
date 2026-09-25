import { describe, it, expect, beforeEach, vi } from 'vitest';
import { renderHook, act, waitFor } from '@testing-library/react';

// specs/0029 WS3.1 — diarization status must survive navigation and never run twice:
//  - the hook rehydrates isRunning/stage/pct from `api_diarization_status` on mount;
//  - `api_diarize_meeting` answering "already running" attaches instead of erroring;
//  - a progress pct that moves backwards within one stage is ignored (the two-ramps
//    oscillation signature of a duplicate run can never reach the UI).

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
  toast: { success: vi.fn(), error: vi.fn(), info: vi.fn() },
}));

import { toast } from 'sonner';
import { useDiarization } from '@/hooks/useDiarization';

const fireProgress = (payload: {
  meeting_id?: string;
  stage: string;
  pct?: number;
}) => {
  act(() => {
    listeners.get('diarization-progress')?.({ payload });
  });
};

beforeEach(() => {
  vi.clearAllMocks();
  listeners.clear();
  invokeMock.mockImplementation((cmd: string) => {
    switch (cmd) {
      case 'api_diarization_status':
        return Promise.resolve(null);
      case 'api_diarization_models_present':
        return Promise.resolve(true);
      case 'api_diarize_meeting':
        return Promise.resolve({ started: true, alreadyRunning: false });
      default:
        return Promise.resolve(null);
    }
  });
});

describe('useDiarization — WS3.1 persistent status & concurrency guard', () => {
  it('rehydrates a live run from api_diarization_status on mount', async () => {
    invokeMock.mockImplementation((cmd: string) =>
      cmd === 'api_diarization_status'
        ? Promise.resolve({ running: true, stage: 'diarizing', progressPct: 40 })
        : Promise.resolve(null),
    );

    const { result } = renderHook(() => useDiarization({ meetingId: 'm1' }));

    await waitFor(() => expect(result.current.isRunning).toBe(true));
    expect(result.current.stage).toBe('diarizing');
    expect(result.current.progressPct).toBe(40);
    expect(invokeMock).toHaveBeenCalledWith('api_diarization_status', {
      meetingId: 'm1',
    });
  });

  it('does not rehydrate from a terminal (not running) status', async () => {
    invokeMock.mockImplementation((cmd: string) =>
      cmd === 'api_diarization_status'
        ? Promise.resolve({ running: false, stage: 'complete', progressPct: 100 })
        : Promise.resolve(null),
    );

    const { result } = renderHook(() => useDiarization({ meetingId: 'm1' }));

    // Give the status promise a tick to resolve, then confirm it stayed idle.
    await act(async () => {});
    expect(result.current.isRunning).toBe(false);
    expect(result.current.stage).toBeNull();
  });

  it('ignores a progress pct that moves backwards within the same stage', async () => {
    const { result } = renderHook(() => useDiarization({ meetingId: 'm1' }));
    await act(async () => {});

    fireProgress({ meeting_id: 'm1', stage: 'diarizing', pct: 76 });
    expect(result.current.progressPct).toBe(76);

    // The interleaved-second-run signature (76→28) must not reach the UI.
    fireProgress({ meeting_id: 'm1', stage: 'diarizing', pct: 28 });
    expect(result.current.progressPct).toBe(76);

    fireProgress({ meeting_id: 'm1', stage: 'diarizing', pct: 85 });
    expect(result.current.progressPct).toBe(85);
  });

  it('lets a stage change legitimately reset the ramp', async () => {
    const { result } = renderHook(() => useDiarization({ meetingId: 'm1' }));
    await act(async () => {});

    fireProgress({ meeting_id: 'm1', stage: 'diarizing', pct: 90 });
    expect(result.current.progressPct).toBe(90);

    fireProgress({ meeting_id: 'm1', stage: 'aligning', pct: 10 });
    expect(result.current.stage).toBe('aligning');
    expect(result.current.progressPct).toBe(10);
  });

  it('marks the run live when a progress event for this meeting arrives', async () => {
    const { result } = renderHook(() => useDiarization({ meetingId: 'm1' }));
    await act(async () => {});
    expect(result.current.isRunning).toBe(false);

    fireProgress({ meeting_id: 'm1', stage: 'diarizing', pct: 5 });
    expect(result.current.isRunning).toBe(true);
  });

  it('ignores progress for a different meeting', async () => {
    const { result } = renderHook(() => useDiarization({ meetingId: 'm1' }));
    await act(async () => {});

    fireProgress({ meeting_id: 'OTHER', stage: 'diarizing', pct: 50 });
    expect(result.current.isRunning).toBe(false);
    expect(result.current.progressPct).toBeNull();
  });

  it('attaches (no error toast, stays running) when the backend says already running', async () => {
    invokeMock.mockImplementation((cmd: string) => {
      switch (cmd) {
        case 'api_diarization_models_present':
          return Promise.resolve(true);
        case 'api_diarize_meeting':
          return Promise.resolve({ started: false, alreadyRunning: true });
        case 'api_diarization_status':
          return Promise.resolve({
            running: true,
            stage: 'diarizing',
            progressPct: 63,
          });
        default:
          return Promise.resolve(null);
      }
    });

    const { result } = renderHook(() => useDiarization({ meetingId: 'm1' }));

    await act(async () => {
      await result.current.identifySpeakers();
    });

    expect(result.current.isRunning).toBe(true);
    expect(result.current.stage).toBe('diarizing');
    expect(result.current.progressPct).toBe(63);
    expect(toast.error).not.toHaveBeenCalled();
    // Exactly one start request went out; the duplicate became an attach.
    expect(
      invokeMock.mock.calls.filter(([cmd]) => cmd === 'api_diarize_meeting'),
    ).toHaveLength(1);
  });

  it('completion resets state so a later re-run starts a fresh ramp', async () => {
    const { result } = renderHook(() => useDiarization({ meetingId: 'm1' }));
    await act(async () => {});

    fireProgress({ meeting_id: 'm1', stage: 'diarizing', pct: 97 });
    act(() => {
      listeners.get('diarization-complete')?.({
        payload: { meeting_id: 'm1', speaker_count: 3 },
      });
    });
    expect(result.current.isRunning).toBe(false);
    expect(result.current.progressPct).toBeNull();

    // A fresh run's low pct must not be swallowed by the old run's 97.
    fireProgress({ meeting_id: 'm1', stage: 'diarizing', pct: 3 });
    expect(result.current.progressPct).toBe(3);
  });
});

// Owner feedback 2026-09-21 — "Is it possible to update the transcript with speakers as
// they get identified… instead of waiting to the very end?" The backend now emits
// `diarization-speakers-updated` once the clusters are persisted and again once the
// voiceprint matches are applied, so the transcript stops being frozen for the last
// stretch of a run.
describe('useDiarization — progressive speaker reveal', () => {
  const fireSpeakersUpdated = (payload: { meeting_id?: string; stage?: string }) => {
    act(() => {
      listeners.get('diarization-speakers-updated')?.({ payload });
    });
  };

  it('refetches the transcript on a speakers-updated event for this meeting', async () => {
    const onComplete = vi.fn();
    renderHook(() => useDiarization({ meetingId: 'm1', onComplete }));
    await waitFor(() => expect(listeners.has('diarization-speakers-updated')).toBe(true));

    fireSpeakersUpdated({ meeting_id: 'm1', stage: 'attributing' });
    expect(onComplete).toHaveBeenCalledTimes(1);

    // Twice per run: clusters, then the applied gallery matches.
    fireSpeakersUpdated({ meeting_id: 'm1', stage: 'labelling' });
    expect(onComplete).toHaveBeenCalledTimes(2);
  });

  it('ignores a speakers-updated event for a different meeting', async () => {
    const onComplete = vi.fn();
    renderHook(() => useDiarization({ meetingId: 'm1', onComplete }));
    await waitFor(() => expect(listeners.has('diarization-speakers-updated')).toBe(true));

    fireSpeakersUpdated({ meeting_id: 'm2', stage: 'attributing' });
    expect(onComplete).not.toHaveBeenCalled();
  });

  // Unlike `diarization-complete`, this fires mid-pass — so it must not end the run or
  // clear the stage, or the button would go idle while sherpa's work is still being written.
  it('does not end the run or show a toast', async () => {
    const onComplete = vi.fn();
    const { result } = renderHook(() => useDiarization({ meetingId: 'm1', onComplete }));
    await waitFor(() => expect(listeners.has('diarization-speakers-updated')).toBe(true));

    fireProgress({ meeting_id: 'm1', stage: 'attributing' });
    expect(result.current.isRunning).toBe(true);

    fireSpeakersUpdated({ meeting_id: 'm1', stage: 'attributing' });
    expect(result.current.isRunning).toBe(true);
    expect(result.current.stage).toBe('attributing');
    expect(toast.success).not.toHaveBeenCalled();
  });

  // The stages reported after sherpa returns carry no percentage, so the button shows a
  // named stage instead of sitting on "Identifying… 100%" (which reads as hung).
  it('reports the post-sherpa stages with no percentage', async () => {
    const { result } = renderHook(() => useDiarization({ meetingId: 'm1' }));
    await waitFor(() => expect(listeners.has('diarization-progress')).toBe(true));

    fireProgress({ meeting_id: 'm1', stage: 'diarizing', pct: 100 });
    expect(result.current.progressPct).toBe(100);

    fireProgress({ meeting_id: 'm1', stage: 'attributing' });
    expect(result.current.stage).toBe('attributing');
    expect(result.current.progressPct).toBeNull();

    fireProgress({ meeting_id: 'm1', stage: 'matching known voices' });
    expect(result.current.stage).toBe('matching known voices');
    expect(result.current.progressPct).toBeNull();

    fireProgress({ meeting_id: 'm1', stage: 'labelling' });
    expect(result.current.stage).toBe('labelling');
    expect(result.current.progressPct).toBeNull();
  });
});

describe('useDiarization — custom start (specs/0078 "Who was on the mic?")', () => {
  it('runs the given start instead of api_diarize_meeting, after the model check', async () => {
    const start = vi.fn().mockResolvedValue({ started: true, alreadyRunning: false });
    const { result } = renderHook(() => useDiarization({ meetingId: 'm1' }));

    await act(async () => {
      await result.current.identifySpeakers(start);
    });

    expect(invokeMock).toHaveBeenCalledWith('api_diarization_models_present');
    expect(start).toHaveBeenCalledTimes(1);
    expect(invokeMock).not.toHaveBeenCalledWith('api_diarize_meeting', expect.anything());
    expect(result.current.isRunning).toBe(true);
  });

  it('a failing start ends the run with an error toast', async () => {
    const start = vi.fn().mockRejectedValue('Couldn\'t save this meeting\'s audio setup');
    const err = vi.spyOn(console, 'error').mockImplementation(() => {});
    const { result } = renderHook(() => useDiarization({ meetingId: 'm1' }));

    await act(async () => {
      await result.current.identifySpeakers(start);
    });

    expect(result.current.isRunning).toBe(false);
    expect(toast.error).toHaveBeenCalled();
    err.mockRestore();
  });
});
