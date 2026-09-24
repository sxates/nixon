import { describe, it, expect, beforeEach, vi } from 'vitest';
import { act, render, screen, waitFor } from '@testing-library/react';

// specs/0074 W2 — the Upcoming list re-reads when a Google sync pass changed the
// cache (`google-calendar-synced`), instead of waiting out its 5-minute interval.

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke }));

const { listeners } = vi.hoisted(() => ({
  listeners: {} as Record<string, (event: unknown) => void>,
}));
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn((event: string, cb: (e: unknown) => void) => {
    listeners[event] = cb;
    return Promise.resolve(() => {
      delete listeners[event];
    });
  }),
}));

vi.mock('next/navigation', () => ({ useRouter: () => ({ push: vi.fn() }) }));
vi.mock('@/contexts/RecordingStateContext', () => ({
  useRecordingState: () => ({ isRecording: false }),
}));
vi.mock('@/components/Sidebar/SidebarProvider', () => ({
  useSidebar: () => ({ handleRecordingToggle: vi.fn() }),
}));

import UpcomingMeetings from '@/components/Calendar/UpcomingMeetings';

let upcoming: unknown[];

beforeEach(() => {
  invoke.mockReset();
  for (const k of Object.keys(listeners)) delete listeners[k];
  upcoming = [];
  invoke.mockImplementation(async (cmd: string) => {
    if (cmd === 'api_get_calendar_access_status') return 'authorized';
    if (cmd === 'api_get_upcoming_meetings') return upcoming;
    return undefined;
  });
});

describe('UpcomingMeetings', () => {
  it('refreshes when google-calendar-synced fires', async () => {
    render(<UpcomingMeetings />);
    await waitFor(() => expect(listeners['google-calendar-synced']).toBeDefined());
    expect(screen.queryByText('Design review')).toBeNull();

    upcoming = [
      {
        id: 'evt-1',
        title: 'Design review',
        startsAt: new Date(Date.now() + 30 * 60_000).toISOString(),
        endsAt: new Date(Date.now() + 60 * 60_000).toISOString(),
        calendarName: 'Work',
        zoomUrl: null,
      },
    ];
    await act(async () => {
      listeners['google-calendar-synced']({ payload: { changed: 1 } });
    });
    expect(await screen.findByText('Design review')).toBeInTheDocument();
  });
});
