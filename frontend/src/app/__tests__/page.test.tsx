import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent, waitFor, within } from '@testing-library/react';

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

// Mutable so individual tests can move "now" relative to the manual item's 15:00–15:30
// window (fix round 2, specs/0069 followup a: `handleRecordManual`'s `startsAt` choice
// depends on whether `now` is before or after the scheduled start). Reset in beforeEach.
let mockNow = new Date('2026-09-20T15:10:00.000Z');

// `useDayAgenda` is mocked directly (rather than driven through `invoke`) so the test
// stays focused on the click → `joinAndRecord` wiring in `handleRecordManual`, not on
// the agenda-loading hook (covered elsewhere).
vi.mock('@/hooks/useDayAgenda', () => ({
  useDayAgenda: () => ({
    items: [manualItem],
    visibleItems: [manualItem],
    calendarStatus: 'granted',
    loaded: true,
    // Decides whether the row shows the Record button (inside the start window) or Prep
    // with "Record now" in its menu (earlier) — and the startsAt the tests below expect.
    now: mockNow,
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
  mockNow = new Date('2026-09-20T15:10:00.000Z');
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

describe('edit → close → add (specs/0069 W3, fix round 1 Finding 2)', () => {
  it('leaves the form blank and creates (not updates) after editing then reopening Add', async () => {
    render(<Home />);

    // Open the manual entry's ⋯ menu and choose Edit.
    const menuTrigger = await screen.findByRole('button', { name: 'Event options' });
    menuTrigger.focus();
    fireEvent.keyDown(menuTrigger, { key: 'Enter' });
    fireEvent.click(await screen.findByText('Edit'));

    // The dialog seeded from the manual entry — title field carries its title, and
    // "Save" (not "Add meeting") is the submit label in edit mode. Scope to the dialog:
    // the header's own "Add meeting" button is a separate, always-present element.
    let dialog = await screen.findByRole('dialog');
    const titleInput = within(dialog).getByLabelText('Title') as HTMLInputElement;
    expect(titleInput.value).toBe('Call with Sam');
    expect(within(dialog).getByRole('button', { name: 'Save' })).toBeInTheDocument();

    // Close without saving.
    fireEvent.click(within(dialog).getByRole('button', { name: 'Cancel' }));
    await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());

    // Reopen via the header's "Add meeting" — a STALE `editing` prop would silently
    // reseed the previous entry's fields and turn this create into an update.
    fireEvent.click(screen.getByRole('button', { name: /add meeting/i }));

    dialog = await screen.findByRole('dialog');
    const reopenedTitle = within(dialog).getByLabelText('Title') as HTMLInputElement;
    expect(reopenedTitle.value).toBe('');
    expect(within(dialog).getByRole('button', { name: 'Add meeting' })).toBeInTheDocument();

    fireEvent.change(reopenedTitle, { target: { value: 'Brand new call' } });
    fireEvent.click(within(dialog).getByRole('button', { name: 'Add meeting' }));

    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith(
        'api_create_manual_meeting',
        expect.objectContaining({ title: 'Brand new call' }),
      ),
    );
    expect(invokeMock).not.toHaveBeenCalledWith(
      'api_update_manual_meeting',
      expect.anything(),
    );
  });
});

describe('handleRecordManual startsAt (fix round 2, specs/0069 followup a)', () => {
  // The manual item is scheduled 15:00–15:30 UTC.

  // Owner feedback 2026-09-24: an hour out the row shows Prep like a calendar meeting, and
  // recording early moved into the row's `…` menu.
  const openRecordNow = async () => {
    const trigger = await screen.findByRole('button', { name: 'Event options' });
    trigger.focus();
    fireEvent.keyDown(trigger, { key: 'Enter' });
    fireEvent.click(await screen.findByText('Record now'));
  };

  it('shows Prep, not the Record button, on a manual entry well in the future', async () => {
    mockNow = new Date('2026-09-20T14:00:00.000Z'); // an hour before the 15:00 start
    render(<Home />);
    expect(await screen.findByText('Prep')).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Record' })).not.toBeInTheDocument();
  });

  it('sends roughly now — not the future scheduled start — as startedAt when recording early', async () => {
    mockNow = new Date('2026-09-20T14:00:00.000Z'); // an hour before the 15:00 start
    render(<Home />);

    await openRecordNow();

    await waitFor(() => {
      const call = invokeMock.mock.calls.find(([cmd]) => cmd === 'api_create_meeting');
      expect(call).toBeDefined();
    });
    const call = invokeMock.mock.calls.find(([cmd]) => cmd === 'api_create_meeting');
    const startedAt = (call?.[1] as { startedAt?: string } | undefined)?.startedAt;
    expect(startedAt).toBeTruthy();
    // NOT the item's scheduled 15:00 start.
    expect(startedAt).not.toBe(manualItem.startTime);
    // Within a few seconds of the mocked "now".
    expect(Math.abs(new Date(startedAt as string).getTime() - mockNow.getTime())).toBeLessThan(5000);
  });

  it('sends the scheduled start as startedAt once it has already passed', async () => {
    // Mid-meeting: the start has passed but the entry hasn't ended. (This used to run 30 min
    // AFTER the window, where Record no longer shows — owner feedback 2026-09-23.)
    mockNow = new Date('2026-09-20T15:10:00.000Z');
    render(<Home />);

    fireEvent.click(await screen.findByRole('button', { name: 'Record' }));

    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith(
        'api_create_meeting',
        expect.objectContaining({ startedAt: manualItem.startTime }),
      ),
    );
  });
});
