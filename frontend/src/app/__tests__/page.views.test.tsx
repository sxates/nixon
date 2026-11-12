import { describe, it, expect, vi, beforeEach } from 'vitest';
import { act, render, screen, fireEvent, within } from '@testing-library/react';

// The Meetings page (Today + All meetings merged). With a calendar connected it switches
// between Agenda / Week / Month / List and remembers the last one; without one it is the
// old All meetings Month / List under the connect-calendar nudge. The view bodies are
// stubbed to markers — their own suites cover what's inside them.

const { searchParams, calendar } = vi.hoisted(() => ({
  searchParams: { current: new URLSearchParams() },
  calendar: { connected: true as boolean | null },
}));
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(async () => []) }));
vi.mock('next/navigation', () => ({
  useRouter: () => ({ push: vi.fn(), replace: vi.fn() }),
  useSearchParams: () => ({ get: (k: string) => searchParams.current.get(k) }),
}));
vi.mock('@/hooks/useCalendarConnected', () => ({ useCalendarConnected: () => calendar.connected }));
vi.mock('@/contexts/RecordingStateContext', () => ({
  useRecordingState: () => ({ isRecording: false }),
}));
vi.mock('@/contexts/TranscriptContext', () => ({
  useTranscripts: () => ({ currentMeetingId: null, meetingTitle: null }),
}));
vi.mock('@/components/Sidebar/SidebarProvider', () => ({
  useSidebar: () => ({ handleRecordingToggle: vi.fn(), activeRecordingMeetingId: null }),
}));
vi.mock('@/contexts/DeferredBacklogProvider', () => ({
  useBacklog: () => ({ view: { pendingCount: 0, processing: false }, startNow: vi.fn() }),
}));
vi.mock('@/contexts/QueueOpenContext', () => ({
  useQueueOpen: () => ({ open: false, setOpen: vi.fn() }),
}));
vi.mock('@/contexts/ImportDialogContext', () => ({ useImportDialog: () => ({ openImportDialog: vi.fn() }) }));
vi.mock('@/components/CommandPalette/SearchMeetingsButton', () => ({ SearchMeetingsButton: () => null }));
vi.mock('framer-motion', () => {
  const passthrough = ({ children, ...props }: { children?: React.ReactNode }) => (
    <div {...props}>{children}</div>
  );
  return { motion: new Proxy({}, { get: () => passthrough }) };
});
vi.mock('@/components/Meetings/AgendaPane', () => ({
  AgendaPane: ({ mode }: { mode: string }) => <div data-testid="pane">{`agenda:${mode}`}</div>,
}));
vi.mock('@/components/Meetings/MonthCalendar', () => ({
  MonthCalendar: ({ events }: { events?: unknown }) => (
    <div data-testid="pane">{events ? 'month:events' : 'month:recorded'}</div>
  ),
}));
vi.mock('@/components/Meetings/MeetingsList', () => ({
  MeetingsList: () => <div data-testid="pane">list</div>,
}));

const { useDayAgendaMock } = vi.hoisted(() => ({ useDayAgendaMock: vi.fn() }));
vi.mock('@/hooks/useDayAgenda', () => ({ useDayAgenda: (mode: unknown) => useDayAgendaMock(mode) }));
const agenda = {
  items: [],
  visibleItems: [],
  loaded: true,
  now: new Date('2026-09-20T15:00:00.000Z'),
  viewDate: '2026-09-20',
  viewIsToday: true,
  weekDays: ['2026-09-14', '2026-09-15', '2026-09-16', '2026-09-17', '2026-09-18', '2026-09-19', '2026-09-20'],
  weekItems: [],
  weekLoaded: true,
  refresh: vi.fn(),
  goToDate: vi.fn(),
  goPrev: vi.fn(),
  goNext: vi.fn(),
  goToday: vi.fn(),
  handleHide: vi.fn(),
};

import Home from '@/app/page';

const pane = () => screen.getByTestId('pane').textContent;
const viewButtons = () =>
  within(screen.getByRole('tablist', { name: 'Meeting view' }))
    .getAllByRole('tab')
    .map((b) => b.textContent?.trim());
// The views are Tabs (docs/design-system.md: tabs switch the view); Radix selects on mousedown.
const pickView = (name: string) =>
  fireEvent.mouseDown(screen.getByRole('tab', { name }), { button: 0 });
const NUDGE = 'Connect your calendar to see your full day';

beforeEach(() => {
  window.localStorage.clear();
  searchParams.current = new URLSearchParams();
  calendar.connected = true;
  useDayAgendaMock.mockReset().mockImplementation((mode: unknown) => ({ ...agenda, viewMode: mode }));
});

describe('Meetings page, calendar connected', () => {
  it('opens on Agenda the first time', () => {
    render(<Home />);
    expect(pane()).toBe('agenda:day');
    expect(viewButtons()).toEqual(['Agenda', 'Week', 'Month', 'List']);
    expect(screen.getByRole('heading', { name: 'Meetings' })).toBeInTheDocument();
    expect(screen.queryByText(NUDGE)).toBeNull();
  });

  it('switches between all four views', () => {
    render(<Home />);
    pickView('Week');
    expect(pane()).toBe('agenda:week');
    pickView('Month');
    expect(pane()).toBe('month:events');
    pickView('List');
    expect(pane()).toBe('list');
    // Month and List have their own data: the agenda reads switch off.
    expect(useDayAgendaMock).toHaveBeenLastCalledWith(null);
    pickView('Agenda');
    expect(pane()).toBe('agenda:day');
  });

  it('remembers the view used last', () => {
    const first = render(<Home />);
    pickView('Month');
    first.unmount();
    render(<Home />);
    expect(pane()).toBe('month:events');
  });

  it("migrates the old Today choice on first visit", () => {
    window.localStorage.setItem('nixon.today.viewMode', 'week');
    render(<Home />);
    expect(pane()).toBe('agenda:week');
  });

  it("keeps Today's ?view=day deep link working", () => {
    window.localStorage.setItem('nixon.meetings.view', 'list');
    searchParams.current = new URLSearchParams('view=day');
    render(<Home />);
    expect(pane()).toBe('agenda:day');
  });

  it('falls back to the no-calendar view when the calendar disconnects on Week', () => {
    const view = render(<Home />);
    pickView('Week');
    calendar.connected = false;
    view.rerender(<Home />);
    expect(pane()).toBe('list');
    expect(viewButtons()).toEqual(['Month', 'List']);
    expect(screen.getByText(NUDGE)).toBeInTheDocument();
    // The choice survives: reconnecting brings Week back.
    calendar.connected = true;
    view.rerender(<Home />);
    expect(pane()).toBe('agenda:week');
  });

  it('waits for the calendar check before drawing a calendar view', async () => {
    calendar.connected = null;
    render(<Home />);
    expect(screen.queryByTestId('pane')).toBeNull();
    expect(screen.queryByText(NUDGE)).toBeNull();
    await act(async () => {});
  });
});

describe('Meetings page, no calendar', () => {
  beforeEach(() => {
    calendar.connected = false;
  });

  it('is the All meetings List under the connect-calendar nudge', () => {
    render(<Home />);
    expect(pane()).toBe('list');
    expect(viewButtons()).toEqual(['Month', 'List']);
    expect(screen.getByText(NUDGE)).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: /add meeting/i })).toBeNull();
  });

  it('switches to a recorded-only Month and remembers it', () => {
    const first = render(<Home />);
    pickView('Month');
    expect(pane()).toBe('month:recorded');
    first.unmount();
    render(<Home />);
    expect(pane()).toBe('month:recorded');
  });
});
