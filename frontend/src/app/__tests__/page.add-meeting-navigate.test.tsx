import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent, waitFor, within } from '@testing-library/react';

/**
 * specs/0069b followup — "when I create a new meeting, take me to its details
 * immediately instead of leaving me on the Today screen." Creating a manual meeting
 * from Today's Add-meeting dialog should navigate to `/meeting-details` on the new
 * meeting's Prep tab (the same place clicking it on the timeline lands, per
 * `routeForItem`'s manual-entry branch) — but editing an existing entry from Today
 * must NOT navigate; it stays put and refreshes the agenda in place.
 */

const { router } = vi.hoisted(() => ({
  router: { push: vi.fn() },
}));

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('next/navigation', () => ({ useRouter: () => router }));
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
vi.mock('@/contexts/DeferredBacklogProvider', () => ({
  useBacklog: () => ({ view: { pendingCount: 0, processing: false }, startNow: vi.fn() }),
}));
vi.mock('@/contexts/QueueOpenContext', () => ({
  useQueueOpen: () => ({ open: false, setOpen: vi.fn() }),
}));
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

const refresh = vi.fn();

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
    refresh,
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
  router.push.mockReset();
  refresh.mockReset();
  invokeMock.mockImplementation((cmd: string) => {
    if (cmd === 'api_create_manual_meeting') return Promise.resolve('new-meeting-1');
    if (cmd === 'api_update_manual_meeting') return Promise.resolve(undefined);
    return Promise.resolve(undefined);
  });
});

describe('creating a manual meeting from Today navigates to it (specs/0069b followup)', () => {
  it('creates and jumps to the new meeting\'s Prep tab, skipping the Today refresh', async () => {
    render(<Home />);

    fireEvent.click(screen.getByRole('button', { name: /add meeting/i }));
    const dialog = await screen.findByRole('dialog');
    fireEvent.change(within(dialog).getByLabelText('Title'), {
      target: { value: 'Brand new call' },
    });
    fireEvent.click(within(dialog).getByRole('button', { name: 'Add meeting' }));

    await waitFor(() =>
      expect(router.push).toHaveBeenCalledWith('/meeting-details?id=new-meeting-1&tab=prep'),
    );
    // Navigating away makes refreshing Today's now-abandoned agenda pointless (and
    // risks a post-unmount state update racing the route change) — it must not fire.
    expect(refresh).not.toHaveBeenCalled();
  });

  it('editing an existing entry from Today does NOT navigate away', async () => {
    render(<Home />);

    const menuTrigger = await screen.findByRole('button', { name: 'Event options' });
    menuTrigger.focus();
    fireEvent.keyDown(menuTrigger, { key: 'Enter' });
    fireEvent.click(await screen.findByText('Edit'));

    const dialog = await screen.findByRole('dialog');
    fireEvent.click(within(dialog).getByRole('button', { name: 'Save' }));

    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('api_update_manual_meeting', expect.anything()));
    expect(router.push).not.toHaveBeenCalled();
    expect(refresh).toHaveBeenCalled();
  });
});
