import { describe, it, expect, vi } from 'vitest';
import { renderHook, waitFor, act } from '@testing-library/react';

// specs/0079: the summary prompt is resolved by the backend from the meeting's stored
// state. The backlog drain's `api_process_transcript` payload must carry no prompt/template
// fields (they would shadow the backend's resolution).

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));

vi.mock('@tauri-apps/api/core', () => ({ invoke }));
// Completes the retranscription wait immediately.
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async (event: string, cb: (e: { payload: unknown }) => void) => {
    if (event === 'retranscription-complete') {
      queueMicrotask(() => cb({ payload: { meeting_id: 'm1' } }));
    }
    return () => {};
  }),
}));
vi.mock('@/lib/safe-listen', () => ({ safeListen: () => () => {} }));
vi.mock('sonner', () => ({
  toast: { success: vi.fn(), error: vi.fn(), warning: vi.fn(), info: vi.fn() },
}));
// Stable references: the hook keys effects on these.
const { sidebar, config } = vi.hoisted(() => ({
  sidebar: {
    refreshMeetings: () => {},
    currentMeeting: null,
    activeRecordingMeetingId: null,
    startSummaryPolling: (_id: string, _pid: string, cb: (p: { status: string }) => void) =>
      void setTimeout(() => cb({ status: 'completed' }), 0),
    stopSummaryPolling: () => {},
  },
  config: {
    betaFeatures: {},
    autoSummary: false,
    transcriptModelConfig: { provider: 'parakeet' },
  },
}));
vi.mock('@/components/Sidebar/SidebarProvider', () => ({ useSidebar: () => sidebar }));
vi.mock('@/contexts/ConfigContext', () => ({ useConfig: () => config }));
vi.mock('@/lib/fetch-all-transcripts', () => ({
  fetchAllTranscripts: vi.fn(async () => [
    { id: 't1', text: 'hello', timestamp: '', audio_start_time: 0 },
  ]),
}));
vi.mock('@/lib/resolve-summary-language', () => ({
  resolveSummaryLanguage: vi.fn(async () => null),
}));

import { useDeferredBacklog } from '@/hooks/useDeferredBacklog';

describe('useDeferredBacklog summary payload', () => {
  it('api_process_transcript carries no templateId / customPrompt', async () => {
    invoke.mockImplementation(async (cmd: string) => {
      switch (cmd) {
        case 'api_get_meeting_metadata':
          return { id: 'm1', title: 'T', folder_path: '/x/m1' };
        case 'api_get_meeting_transcripts':
          return { total_count: 4, transcripts: [] };
        case 'api_list_deferred_meetings':
          return [];
        case 'api_get_model_config':
          return { provider: 'ollama', model: 'gemma2:2b' };
        case 'api_process_transcript':
          return { process_id: 'proc-1' };
        default:
          return null;
      }
    });
    const { result } = renderHook(() => useDeferredBacklog());
    await act(async () => {
      await result.current.enqueueMeeting('m1', { force: true });
    });
    // Let the drain run to completion inside act so no state update escapes it.
    await waitFor(() =>
      expect(result.current.view.items.find((i) => i.meeting.id === 'm1')?.status).toBe('done'),
    );
    const call = invoke.mock.calls.find((c) => c[0] === 'api_process_transcript');
    const payload = call![1] as Record<string, unknown>;
    expect(payload.meetingId).toBe('m1');
    expect(payload).not.toHaveProperty('templateId');
    expect(payload).not.toHaveProperty('customPrompt');
  });
});
