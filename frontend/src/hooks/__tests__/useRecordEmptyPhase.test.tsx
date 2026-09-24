import { describe, it, expect, vi, beforeEach } from 'vitest';
import { renderHook } from '@testing-library/react';
import { RecordingStatus } from '@/contexts/RecordingStateContext';

let status = RecordingStatus.IDLE;
vi.mock('@/contexts/RecordingStateContext', async (orig) => ({
  ...(await orig<typeof import('@/contexts/RecordingStateContext')>()),
  useRecordingState: () => ({ status }),
}));
vi.mock('@/lib/resume-recording', () => ({ isResumeArmedOrInFlight: () => false }));

import { useRecordEmptyPhase } from '../useRecordEmptyPhase';

// Owner feedback 2026-09-23: "Welcome to Nixon!" flashed after REC and while saving.
describe('useRecordEmptyPhase', () => {
  beforeEach(() => {
    status = RecordingStatus.IDLE;
    window.sessionStorage.clear();
  });

  it('reads "starting" from the first render when REC was pressed on another page', () => {
    window.sessionStorage.setItem('autoStartRecording', 'true');
    const { result } = renderHook(() => useRecordEmptyPhase());
    expect(result.current).toBe('starting');
  });

  it('reads "starting" while the start is initialising', () => {
    status = RecordingStatus.STARTING;
    expect(renderHook(() => useRecordEmptyPhase()).result.current).toBe('starting');
  });

  it.each([
    RecordingStatus.STOPPING,
    RecordingStatus.PROCESSING_TRANSCRIPTS,
    RecordingStatus.SAVING,
    RecordingStatus.COMPLETED,
  ])('reads "saving" while %s', (s) => {
    status = s;
    expect(renderHook(() => useRecordEmptyPhase()).result.current).toBe('saving');
  });

  it('is idle when nothing was requested', () => {
    expect(renderHook(() => useRecordEmptyPhase()).result.current).toBeUndefined();
  });
});
