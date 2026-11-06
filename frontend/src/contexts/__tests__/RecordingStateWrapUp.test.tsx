import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { renderHook, act } from '@testing-library/react';

// specs/0083 W2b — `meeting-wrap-up` drives the post-stop RecordingStatus, so the rail and
// the sidebar reflect a wrap-up from any stop source without anyone navigating, and a reload
// mid-wrap-up picks it up from `api_get_wrap_up_state`.

const h = vi.hoisted(() => ({
  listeners: new Map<string, (event: { payload: unknown }) => void>(),
  invoke: vi.fn(),
}));

vi.mock('@/services/recordingService', () => ({
  recordingService: {
    getRecordingState: vi.fn().mockResolvedValue({
      is_recording: false,
      is_paused: false,
      is_active: false,
      recording_duration: null,
      active_duration: null,
    }),
    onRecordingStarted: vi.fn().mockResolvedValue(() => {}),
    onRecordingStopped: vi.fn().mockResolvedValue(() => {}),
    onRecordingPaused: vi.fn().mockResolvedValue(() => {}),
    onRecordingResumed: vi.fn().mockResolvedValue(() => {}),
  },
}));
vi.mock('@/lib/safe-listen', () => ({
  makeSafeUnlisten: () => () => {},
  safeListen: (event: string, handler: (event: { payload: unknown }) => void) => {
    h.listeners.set(event, handler);
    return () => h.listeners.delete(event);
  },
}));
vi.mock('@tauri-apps/api/core', () => ({ invoke: h.invoke }));

import {
  RecordingStateProvider,
  useRecordingState,
  useWrapUp,
  RecordingStatus,
} from '@/contexts/RecordingStateContext';

const wrapper = ({ children }: { children: React.ReactNode }) => (
  <RecordingStateProvider>{children}</RecordingStateProvider>
);

const wrapUp = (stage: string, id = 1) => ({
  id,
  meetingId: 'm-1',
  source: 'tray',
  stage,
  title: null,
  transcriptionIncomplete: false,
  reason: null,
  message: stage === 'failed' ? 'Couldn’t save the meeting' : null,
  toast: null,
  failedStage: stage === 'failed' ? 'saving' : null,
});

const flush = () => act(async () => { await Promise.resolve(); await Promise.resolve(); });

function emit(stage: string) {
  act(() => h.listeners.get('meeting-wrap-up')!({ payload: wrapUp(stage) }));
}

beforeEach(() => {
  vi.useFakeTimers();
  h.listeners.clear();
  h.invoke.mockResolvedValue(null);
});
afterEach(() => {
  vi.useRealTimers();
  vi.clearAllMocks();
});

describe('wrap-up stages map to RecordingStatus', () => {
  it.each([
    ['stopping', RecordingStatus.PROCESSING_TRANSCRIPTS],
    ['draining', RecordingStatus.PROCESSING_TRANSCRIPTS],
    ['saving', RecordingStatus.SAVING],
    ['saved', RecordingStatus.COMPLETED],
    ['discarded', RecordingStatus.IDLE],
    ['failed', RecordingStatus.ERROR],
  ])('%s → %s', async (stage, status) => {
    const { result } = renderHook(() => useRecordingState(), { wrapper });
    await flush();
    emit(stage);
    expect(result.current.status).toBe(status);
    expect(result.current.wrapUp?.stage).toBe(stage);
  });

  it('saved settles to idle; the stages after it leave the status alone', async () => {
    const { result } = renderHook(() => useRecordingState(), { wrapper });
    await flush();
    emit('saved');
    act(() => vi.advanceTimersByTime(2000));
    expect(result.current.status).toBe(RecordingStatus.IDLE);
    emit('diarizing');
    emit('summarizing');
    emit('done');
    expect(result.current.status).toBe(RecordingStatus.IDLE);
  });

  it('a stop the webview never asked for (tray) still ends the recording in the UI', async () => {
    const { result } = renderHook(() => useRecordingState(), { wrapper });
    await flush();
    emit('stopping');
    expect(result.current.isRecording).toBe(false);
    expect(result.current.isProcessing).toBe(true);
  });
});

describe('hydration (a reload, or a window shown mid-wrap-up)', () => {
  it('picks up a wrap-up that is still saving', async () => {
    h.invoke.mockImplementation(async (cmd: string) =>
      cmd === 'api_get_wrap_up_state' ? wrapUp('saving') : null,
    );
    const { result } = renderHook(() => useRecordingState(), { wrapper });
    await flush();
    expect(result.current.status).toBe(RecordingStatus.SAVING);
    expect(result.current.wrapUp?.stage).toBe('saving');
  });

  it('does not replay a finished one as a status', async () => {
    h.invoke.mockImplementation(async (cmd: string) =>
      cmd === 'api_get_wrap_up_state' ? wrapUp('failed') : null,
    );
    const { result } = renderHook(() => useWrapUp(), { wrapper });
    await flush();
    expect(result.current?.stage).toBe('failed');
  });
});
