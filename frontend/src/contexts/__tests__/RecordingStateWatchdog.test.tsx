import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { renderHook, act } from '@testing-library/react';

// specs/0019 WS6.2 — the floating recording bar shows while status is a transient
// finalize state (STOPPING/PROCESSING/SAVING). If the stop handler ever fails to reach a
// terminal state the bar would linger forever, so the context runs a watchdog that forces
// IDLE after a generous bound. These lock that backstop (and that it doesn't fire early /
// for a live recording).

const { getRecordingState } = vi.hoisted(() => ({
  getRecordingState: vi.fn().mockResolvedValue({
    is_recording: false,
    is_paused: false,
    is_active: false,
    recording_duration: null,
    active_duration: null,
  }),
}));

vi.mock('@/services/recordingService', () => ({
  recordingService: {
    getRecordingState,
    onRecordingStarted: vi.fn().mockResolvedValue(() => {}),
    onRecordingStopped: vi.fn().mockResolvedValue(() => {}),
    onRecordingPaused: vi.fn().mockResolvedValue(() => {}),
    onRecordingResumed: vi.fn().mockResolvedValue(() => {}),
  },
}));
vi.mock('@/lib/safe-listen', () => ({
  makeSafeUnlisten: () => () => {},
  safeListen: () => () => {},
}));

import {
  RecordingStateProvider,
  useRecordingState,
  RecordingStatus,
} from '@/contexts/RecordingStateContext';

const wrapper = ({ children }: { children: React.ReactNode }) => (
  <RecordingStateProvider>{children}</RecordingStateProvider>
);

beforeEach(() => {
  vi.useFakeTimers();
});
afterEach(() => {
  vi.useRealTimers();
  vi.clearAllMocks();
});

describe('RecordingState finalize watchdog (WS6.2)', () => {
  it('forces a stuck PROCESSING status to IDLE after the bound (bar clears)', async () => {
    const { result } = renderHook(() => useRecordingState(), { wrapper });

    act(() => {
      result.current.setStatus(RecordingStatus.PROCESSING_TRANSCRIPTS, 'stuck');
    });
    expect(result.current.isProcessing).toBe(true);

    // Just before the bound: still stuck.
    act(() => {
      vi.advanceTimersByTime(119000);
    });
    expect(result.current.isProcessing).toBe(true);

    // Past the bound: forced to IDLE → the bar's predicate goes false.
    act(() => {
      vi.advanceTimersByTime(2000);
    });
    expect(result.current.status).toBe(RecordingStatus.IDLE);
    expect(result.current.isProcessing).toBe(false);
    expect(result.current.isSaving).toBe(false);
    expect(result.current.isStopping).toBe(false);
  });

  it('does not fire once a terminal state is reached in time', () => {
    const { result } = renderHook(() => useRecordingState(), { wrapper });

    act(() => {
      result.current.setStatus(RecordingStatus.SAVING);
    });
    act(() => {
      vi.advanceTimersByTime(5000);
      result.current.setStatus(RecordingStatus.COMPLETED); // saved before the bound
    });
    act(() => {
      vi.advanceTimersByTime(120000);
    });
    // The watchdog cleared when status left the transient set; COMPLETED is preserved.
    expect(result.current.status).toBe(RecordingStatus.COMPLETED);
  });

  it('never arms while actively recording', () => {
    const { result } = renderHook(() => useRecordingState(), { wrapper });
    act(() => {
      result.current.setStatus(RecordingStatus.RECORDING);
      vi.advanceTimersByTime(120000);
    });
    expect(result.current.status).toBe(RecordingStatus.RECORDING);
  });
});
