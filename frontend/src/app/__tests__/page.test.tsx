import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/react';

// specs/0069 W3 — recording a manually added meeting must go through the SAME adoption
// path as calendar Join & Record: `joinAndRecord` stashes the event's `calendarEventId`
// and passes it to `api_create_meeting`, which matches on it to adopt the existing
// `scheduled` prep row (specs/0036) instead of minting a second, empty meeting. A manual
// item's own `id` IS its meeting id (not its `nixon-manual:` event id) — passing that
// instead would match nothing and silently lose whatever prep was written. This is the
// regression the sabotage check in the task-5 report is aimed at.

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('next/navigation', () => ({ useRouter: () => ({ push: vi.fn() }) }));
vi.mock('@/contexts/RecordingStateContext', () => ({
  useRecordingState: () => ({ isRecording: false }),
}));
vi.mock('@/contexts/TranscriptContext', () => ({
  useTranscripts: () => ({ currentMeetingId: null, meetingTitle: null }),
}));
vi.mock('@/components/Sidebar/SidebarProvider', () => ({
  useSidebar: () => ({
    handleRecordingToggle: vi.fn(),
    activeRecordingMeetingId: null,
  }),
}));
// TodayHeader renders ProcessMeetingsButton, which needs these two contexts — no backlog
// and no queue, so the header just renders its plain buttons.
vi.mock('@/contexts/DeferredBacklogProvider', () => ({
  useBacklog: () => ({ view: { pendingCount: 0, processing: false }, startNow: vi.fn() }),
}));
vi.mock('@/contexts/QueueOpenContext', () => ({
  useQueueOpen: () => ({ open: false, setOpen: vi.fn() }),
}));
// framer-motion `motion.div` → passthrough so jsdom renders children immediately (same
// cache-per-key trick as the All Meetings page test — a fresh component type per access
// would remount the subtree on every render).
vi.mock('framer-motion', () => {
  const cache = new Map<string | symbol, unknown>();
  const passthrough =
    ({ children, ...props }: { children?: React.ReactNode }) => <div {...props}>{children}</div>;
  return {
    motion: new Proxy(
      {},
      {
        get: (_target, key) => {
          if (!cache.has(key)) cache.set(key, passthrough);
          return cache.get(key);
        },
      },
    ),
  };
});

const manualItem = {
  id: 'meeting-1',
  title: 'Call with Sam',
  startTime: '2026-09-20T15:00:00.000Z',
  endTime: '2026-09-20T15:30:00.000Z',
  source: 'manual' as const,
  zoomUrl: null,
  attendees: [],
  attendeeCount: 0,
  meetingId: 'meeting-1',
  status: { recorded: false, transcribed: false, summarized: false, speakersIdentified: false },
  dismissed: false,
  calendarEventId: 'nixon-manual:meeting-1',
};

// `useDayAgenda` is mocked directly (rather than driven through `invoke`) so the test
// stays focused on the click → `joinAndRecord` wiring in `handleRecordManual`, not on
// the agenda-loading hook (covered elsewhere).
vi.mock('@/hooks/useDayAgenda', () => ({
  useDayAgenda: () => ({
    items: [manualItem],
    visibleItems: [manualItem],
    calendarStatus: 'granted',
    loaded: true,
    now: new Date('2026-09-20T14:00:00.000Z'),
    viewDate: '2026-09-20',
    viewMode: 'day',
    viewIsToday: true,
    // Real Mon-start week keys (page.tsx indexes weekDays[0]/[6] for its range label
    // even in day mode, where weekItems/weekDays are otherwise unused).
    weekDays: [
      '2026-09-14',
      '2026-09-15',
      '2026-09-16',
      '2026-09-17',
      '2026-09-18',
      '2026-09-19',
      '2026-09-20',
    ],
    weekItems: [[manualItem]],
    weekLoaded: true,
    refresh: vi.fn(),
    goToDate: vi.fn(),
    goPrev: vi.fn(),
    goNext: vi.fn(),
    goToday: vi.fn(),
    switchMode: vi.fn(),
    openDay: vi.fn(),
    handleHide: vi.fn(),
  }),
}));

import { invoke } from '@tauri-apps/api/core';
import Home from '@/app/page';

const invokeMock = vi.mocked(invoke);

beforeEach(() => {
  invokeMock.mockReset();
  invokeMock.mockImplementation((cmd: string) => {
    if (cmd === 'api_create_meeting') return Promise.resolve({ meeting_id: 'promoted-1' });
    return Promise.resolve(undefined);
  });
  window.sessionStorage.clear();
});

describe('recording a manual entry (specs/0069 W3)', () => {
  it('records a manual entry against its own row, not a new one', async () => {
    render(<Home />);

    fireEvent.click(await screen.findByRole('button', { name: 'Record' }));

    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith(
        'api_create_meeting',
        expect.objectContaining({ calendarEventId: 'nixon-manual:meeting-1' }),
      ),
    );
    // Never the meeting's own id — that would match no scheduled row and mint a
    // second, empty meeting beside the one the user prepped.
    expect(invokeMock).not.toHaveBeenCalledWith(
      'api_create_meeting',
      expect.objectContaining({ calendarEventId: 'meeting-1' }),
    );
  });
});
