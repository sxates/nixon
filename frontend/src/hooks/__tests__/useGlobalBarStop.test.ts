import { describe, it, expect, vi, beforeEach } from 'vitest';
import { renderHook, waitFor } from '@testing-library/react';

const { recordingState, stopRecordingMock, toastErrorMock } = vi.hoisted(() => ({
  recordingState: { isRecording: true, isStopping: false },
  stopRecordingMock: vi.fn(),
  toastErrorMock: vi.fn(),
}));
vi.mock('@tauri-apps/api/path', () => ({ appDataDir: () => Promise.resolve('/data') }));
vi.mock('sonner', () => ({ toast: { error: toastErrorMock } }));
vi.mock('@/contexts/RecordingStateContext', () => ({ useRecordingState: () => recordingState }));
vi.mock('@/services/recordingService', () => ({
  recordingService: { stopRecording: stopRecordingMock },
}));

import { useGlobalBarStop } from '@/hooks/useGlobalBarStop';

const setIsStopping = vi.fn();
const handleRecordingStop = vi.fn(async () => {});

const mountAndStop = () => {
  sessionStorage.setItem('stopRecordingOnLoad', 'true');
  return renderHook(() => useGlobalBarStop({ setIsStopping, handleRecordingStop }));
};

beforeEach(() => {
  vi.clearAllMocks();
  sessionStorage.clear();
  Object.assign(recordingState, { isRecording: true, isStopping: false });
  stopRecordingMock.mockResolvedValue(undefined);
  handleRecordingStop.mockResolvedValue(undefined);
});

describe('useGlobalBarStop', () => {
  it('runs the full stop: stop_recording then the post-stop save flow', async () => {
    mountAndStop();
    await waitFor(() => expect(handleRecordingStop).toHaveBeenCalledWith(true));
    expect(stopRecordingMock).toHaveBeenCalledTimes(1);
    expect(setIsStopping).toHaveBeenCalledWith(true);
    expect(toastErrorMock).not.toHaveBeenCalled();
  });

  // The retired RecordingControls.stopRecordingAction fell back to the LOCAL post-processing
  // (callApi=false) whenever the command failed for anything but the benign already-stopped
  // case. This hook is now the only stop path, so losing that lost the meeting.
  it('falls back to local post-processing when stop_recording fails', async () => {
    stopRecordingMock.mockRejectedValue(new Error('audio device disappeared'));
    mountAndStop();
    await waitFor(() => expect(handleRecordingStop).toHaveBeenCalledWith(false));
    expect(handleRecordingStop).not.toHaveBeenCalledWith(true);
    expect(toastErrorMock).toHaveBeenCalledWith('Failed to stop recording', {
      description: 'audio device disappeared',
    });
    expect(setIsStopping).toHaveBeenLastCalledWith(false);
  });

  it('stays quiet (and runs no fallback) for the benign already-stopped error', async () => {
    stopRecordingMock.mockRejectedValue(new Error('No recording in progress'));
    mountAndStop();
    await waitFor(() => expect(stopRecordingMock).toHaveBeenCalled());
    await waitFor(() => expect(setIsStopping).toHaveBeenCalledWith(false));
    expect(handleRecordingStop).not.toHaveBeenCalled();
    expect(toastErrorMock).not.toHaveBeenCalled();
  });
});
