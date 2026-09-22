import { describe, it, expect, vi, beforeEach } from 'vitest';
import { renderHook, waitFor } from '@testing-library/react';

// The bug: `enqueueMeeting` refused EVERY meeting.
//
// It read `meeting.folder_path` from `storageService.getMeeting`, which is
// `api_get_meeting` -> `MeetingDetails` — a struct with no folder-path field at all (id,
// title, created_at, updated_at, origin, calendarEventId, transcripts). So the guard below
// it returned `{accepted: false, reason: 'no-folder-path'}` 100% of the time:
//
//   - every live->defer->live stop handoff failed, so the user got "process this manually"
//     on every such recording;
//   - the per-meeting "Process now" button bailed before dispatching or draining, so it
//     silently did nothing.
//
// The periodic refresh path was unaffected, which is why deferred meetings still eventually
// processed — `api_list_deferred_meetings` returns `DeferredMeeting` with
// `rename_all = "camelCase"`, matching its TS type. That refresh is also the real reason a
// reported stop took 3min19s to summarize: not retranscription cost, but a failed handoff
// followed by the refresh eventually noticing the marker.
//
// These mocks return the REAL payload shapes of each command, which is the whole point: the
// previous absence of a test here let a permanently-false guard ship.

const { invoke, listeners } = vi.hoisted(() => ({
  invoke: vi.fn(),
  listeners: new Map<string, (e: { payload: unknown }) => void>(),
}));

vi.mock('@tauri-apps/api/core', () => ({ invoke }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => {}) }));
vi.mock('@/lib/safe-listen', () => ({
  safeListen: (event: string, cb: (e: { payload: unknown }) => void) => {
    listeners.set(event, cb);
    return () => listeners.delete(event);
  },
}));
vi.mock('sonner', () => ({ toast: { success: vi.fn(), error: vi.fn(), warning: vi.fn(), info: vi.fn() } }));
vi.mock('@/components/Sidebar/SidebarProvider', () => ({
  useSidebar: () => ({ refreshMeetings: vi.fn(), currentMeeting: null, activeRecordingMeetingId: null }),
}));
vi.mock('@/contexts/ConfigContext', () => ({
  useConfig: () => ({ betaFeatures: {}, autoSummary: false }),
}));
vi.mock('@/lib/fetch-all-transcripts', () => ({ fetchAllTranscripts: vi.fn(async () => []) }));

import { useDeferredBacklog } from '@/hooks/useDeferredBacklog';

const FOLDER = '/Users/x/Movies/nixon-recordings-dev/Meeting 21_09_26';

/**
 * The real shapes. `api_get_meeting` (MeetingDetails) genuinely has no folder path;
 * `api_get_meeting_metadata` (MeetingMetadata) has `folder_path`, snake_case.
 */
function mockCommands(over: Record<string, unknown> = {}) {
  invoke.mockImplementation(async (cmd: string) => {
    if (cmd in over) return over[cmd];
    switch (cmd) {
      case 'api_get_meeting':
        return {
          id: 'm1',
          title: 'Meeting 21_09_26',
          created_at: '2026-09-22T03:53:45Z',
          updated_at: '2026-09-22T03:55:00Z',
          origin: 'recorded',
          transcripts: [],
        };
      case 'api_get_meeting_metadata':
        return {
          id: 'm1',
          title: 'Meeting 21_09_26',
          created_at: '2026-09-22T03:53:45Z',
          updated_at: '2026-09-22T03:55:00Z',
          folder_path: FOLDER,
          origin: 'recorded',
        };
      case 'api_get_meeting_transcripts':
        return { total_count: 4, transcripts: [] };
      case 'api_list_deferred_meetings':
        return [];
      case 'api_get_power_state':
        return { onBattery: false };
      default:
        return null;
    }
  });
}

beforeEach(() => {
  invoke.mockReset();
  listeners.clear();
  mockCommands();
});

describe('enqueueMeeting accepts a real recorded meeting', () => {
  it('accepts it — the guard must not refuse every meeting', async () => {
    const { result } = renderHook(() => useDeferredBacklog());
    const outcome = await result.current.enqueueMeeting('m1', { force: true });
    expect(outcome).toEqual({ accepted: true });
  });

  it('puts the meeting on the queue so the UI can show it', async () => {
    const { result } = renderHook(() => useDeferredBacklog());
    await result.current.enqueueMeeting('m1', { force: true });
    await waitFor(() =>
      expect(result.current.view.items.some((i) => i.meeting.id === 'm1')).toBe(true),
    );
    expect(result.current.view.items.find((i) => i.meeting.id === 'm1')?.meeting.folderPath).toBe(
      FOLDER,
    );
  });

  it('asks a command that actually carries a folder path', async () => {
    const { result } = renderHook(() => useDeferredBacklog());
    await result.current.enqueueMeeting('m1', { force: true });
    const asked = invoke.mock.calls.map((c) => c[0] as string);
    expect(asked).toContain('api_get_meeting_metadata');
  });

  // The guard itself is still right and must keep working: a notes-only meeting has no
  // folder to process, and dropping it on the floor silently is what spec 0051 WS2 fixed.
  it('still refuses a meeting that genuinely has no folder', async () => {
    mockCommands({
      api_get_meeting_metadata: {
        id: 'm1',
        title: 'Notes only',
        created_at: '2026-09-22T03:53:45Z',
        updated_at: '2026-09-22T03:53:45Z',
        origin: 'notes_only',
      },
    });
    const { result } = renderHook(() => useDeferredBacklog());
    const outcome = await result.current.enqueueMeeting('m1', { force: true });
    expect(outcome).toEqual({ accepted: false, reason: 'no-folder-path' });
  });

  it('refuses when the metadata lookup itself fails, rather than throwing', async () => {
    mockCommands();
    invoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'api_get_meeting_metadata') throw new Error('meeting not found');
      if (cmd === 'api_list_deferred_meetings') return [];
      if (cmd === 'api_get_power_state') return { onBattery: false };
      return null;
    });
    const { result } = renderHook(() => useDeferredBacklog());
    const outcome = await result.current.enqueueMeeting('gone', { force: true });
    expect(outcome.accepted).toBe(false);
  });
});
