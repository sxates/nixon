import { describe, it, expect, vi, beforeEach } from 'vitest';
import { renderHook, waitFor } from '@testing-library/react';

// specs/0084 W1 review fix: the deferred backlog auto-drains on every launch on AC power. For
// a meeting whose speaker-identification pass crashed the app, Rust now refuses the drain's
// automatic pass (`skippedAfterCrash`) and sends no event. The drain must treat that as
// "skip speaker identification": not wait out the 15-minute ceiling, still summarize, and
// still clear the defer marker so the meeting leaves the backlog.

const { invoke, eventHandlers } = vi.hoisted(() => ({
  invoke: vi.fn(),
  eventHandlers: new Map<string, Set<(e: { payload: unknown }) => void>>(),
}));

vi.mock('@tauri-apps/api/core', () => ({ invoke }));
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async (event: string, cb: (e: { payload: unknown }) => void) => {
    const set = eventHandlers.get(event) ?? new Set();
    set.add(cb);
    eventHandlers.set(event, set);
    return () => set.delete(cb);
  }),
}));
vi.mock('@/lib/safe-listen', () => ({ safeListen: () => () => {} }));
vi.mock('sonner', () => ({
  toast: { success: vi.fn(), error: vi.fn(), warning: vi.fn(), info: vi.fn() },
}));
vi.mock('@/components/Sidebar/SidebarProvider', () => ({
  useSidebar: () => ({
    refreshMeetings: vi.fn(),
    currentMeeting: null,
    activeRecordingMeetingId: null,
  }),
}));
vi.mock('@/contexts/ConfigContext', () => ({
  useConfig: () => ({ betaFeatures: {}, autoSummary: false }),
}));
vi.mock('@/lib/fetch-all-transcripts', () => ({ fetchAllTranscripts: vi.fn(async () => []) }));

import { useDeferredBacklog } from '@/hooks/useDeferredBacklog';

function emit(event: string, payload: unknown) {
  eventHandlers.get(event)?.forEach((cb) => cb({ payload }));
}

beforeEach(() => {
  invoke.mockReset();
  eventHandlers.clear();
  invoke.mockImplementation(async (cmd: string) => {
    switch (cmd) {
      case 'api_get_meeting_metadata':
        return {
          id: 'm1',
          title: 'Crashed meeting',
          created_at: '2026-11-05T09:00:00Z',
          updated_at: '2026-11-05T09:30:00Z',
          folder_path: '/recordings/crashed',
          origin: 'recorded',
        };
      case 'api_get_meeting_transcripts':
        return { total_count: 0, transcripts: [] };
      case 'api_list_deferred_meetings':
        return [];
      case 'api_get_power_state':
        return { onBattery: false };
      case 'start_retranscription_command':
        setTimeout(() => emit('retranscription-complete', { meeting_id: 'm1' }), 0);
        return null;
      case 'api_get_diarization_enabled':
      case 'api_diarization_models_present':
        return true;
      case 'api_diarize_meeting':
        return { started: false, alreadyRunning: false, skippedAfterCrash: true };
      default:
        return null;
    }
  });
});

describe('the drain on a meeting whose speaker identification crashed', () => {
  it('asks for an automatic pass, skips it on refusal, and still clears the marker', async () => {
    const { result } = renderHook(() => useDeferredBacklog());
    await result.current.enqueueMeeting('m1', { force: true });

    await waitFor(
      () =>
        expect(invoke).toHaveBeenCalledWith('api_set_meeting_processing_mode', {
          meetingId: 'm1',
          mode: null,
        }),
      { timeout: 3000 },
    );
    expect(invoke).toHaveBeenCalledWith('api_diarize_meeting', {
      meetingId: 'm1',
      automatic: true,
    });
  });
});
