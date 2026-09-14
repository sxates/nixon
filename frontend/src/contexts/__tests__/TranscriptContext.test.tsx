import React from 'react';
import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { renderHook, act } from '@testing-library/react';

// specs/0023 L3 — invariant 6 (buffer isolation between recordings). Each new
// recording must get its OWN meeting identity: the provider mints a fresh
// `currentMeetingId` on every `recording-started`, and the transcript buffer effect
// is keyed on `[currentMeetingId]` so it resets per session — segments from meeting A
// can't bleed into meeting B. We assert the trigger (a distinct id per recording)
// rather than the internal Map, which keeps the test robust.

const { startedCbRef, onRecordingStarted, onRecordingStopped, onTranscriptUpdate, getRecordingMeetingName } =
  vi.hoisted(() => {
    const startedCbRef: { current: null | (() => Promise<void>) } = { current: null };
    return {
      startedCbRef,
      onRecordingStarted: vi.fn(async (cb: () => Promise<void>) => {
        startedCbRef.current = cb;
        return () => {};
      }),
      onRecordingStopped: vi.fn(async () => () => {}),
      onTranscriptUpdate: vi.fn(async () => () => {}),
      getRecordingMeetingName: vi.fn(async () => 'Test Meeting'),
    };
  });

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn().mockResolvedValue('') }));
vi.mock('@/lib/safe-listen', () => ({
  safeListen: vi.fn(() => () => {}),
  makeSafeUnlisten: vi.fn((fn: () => void) => fn ?? (() => {})),
}));
vi.mock('@/services/indexedDBService', () => ({
  indexedDBService: {
    init: vi.fn().mockResolvedValue(undefined),
    saveMeetingMetadata: vi.fn().mockResolvedValue(undefined),
    getMeetingMetadata: vi.fn().mockResolvedValue(null),
    saveTranscript: vi.fn().mockResolvedValue(undefined),
    markMeetingSaved: vi.fn().mockResolvedValue(undefined),
  },
}));
vi.mock('@/services/recordingService', () => ({
  recordingService: { onRecordingStarted, onRecordingStopped, getRecordingMeetingName },
}));
vi.mock('@/services/transcriptService', () => ({
  transcriptService: { onTranscriptUpdate },
}));
vi.mock('@/contexts/RecordingStateContext', () => ({
  useRecordingState: () => ({
    status: 'idle',
    setStatus: vi.fn(),
    isStopping: false,
    isProcessing: false,
    isSaving: false,
  }),
  RecordingStatus: { IDLE: 'idle', RECORDING: 'recording' },
}));

import { TranscriptProvider, useTranscripts } from '@/contexts/TranscriptContext';

const wrapper = ({ children }: { children: React.ReactNode }) => (
  <TranscriptProvider>{children}</TranscriptProvider>
);

describe('TranscriptContext — per-recording meeting identity (WS6.7 invariant 6)', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    startedCbRef.current = null;
    sessionStorage.clear();
    // Monotonic clock so successive `meeting-${Date.now()}` ids are distinct.
    let clock = 1_000_000;
    vi.spyOn(Date, 'now').mockImplementation(() => (clock += 1000));
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it('starts with no current meeting', () => {
    const { result } = renderHook(() => useTranscripts(), { wrapper });
    expect(result.current.currentMeetingId).toBeNull();
  });

  it('mints a fresh meeting id on recording-started and persists it', async () => {
    const { result } = renderHook(() => useTranscripts(), { wrapper });

    // Flush the async listener setup so the recording-started callback is captured.
    await act(async () => {});
    expect(startedCbRef.current).toBeTypeOf('function');

    await act(async () => {
      await startedCbRef.current!();
    });

    const firstId = result.current.currentMeetingId;
    expect(firstId).toMatch(/^meeting-/);
    expect(sessionStorage.getItem('indexeddb_current_meeting_id')).toBe(firstId);
  });

  it('assigns a DISTINCT meeting id to a second recording (buffer reset between sessions)', async () => {
    const { result } = renderHook(() => useTranscripts(), { wrapper });

    await act(async () => {});
    await act(async () => {
      await startedCbRef.current!();
    });
    const firstId = result.current.currentMeetingId;

    // currentMeetingId changed -> the [currentMeetingId] effect re-subscribes and
    // re-captures the callback; fire it again for the next recording.
    await act(async () => {});
    await act(async () => {
      await startedCbRef.current!();
    });
    const secondId = result.current.currentMeetingId;

    expect(firstId).toMatch(/^meeting-/);
    expect(secondId).toMatch(/^meeting-/);
    expect(secondId).not.toBe(firstId);
  });
});
