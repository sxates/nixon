import { describe, it, expect, vi, beforeEach } from 'vitest';
import { act, renderHook, waitFor } from '@testing-library/react';

// specs/0069 W4 — Today defaults to the List view on a machine with no calendar
// connected, rather than an 8am-6pm hour grid with nothing in it. The default must
// never fight an explicit choice: precedence is `?view=` -> the persisted
// `nixon.today.viewMode` preference -> (connected ? 'day' : 'list'). The connection
// check (`isAnyCalendarConnected`) resolves asynchronously, so a fresh no-calendar
// profile briefly renders Day before settling to List — that's acceptable. What must
// never happen is the async check overwriting a preference the user (or the URL)
// already expressed.

const { searchParamsGetMock } = vi.hoisted(() => ({
  searchParamsGetMock: vi.fn((_key: string) => null as string | null),
}));
vi.mock('next/navigation', () => ({
  useRouter: () => ({ push: vi.fn(), replace: vi.fn() }),
  useSearchParams: () => ({ get: searchParamsGetMock }),
}));

const { getDayAgendaMock } = vi.hoisted(() => ({
  getDayAgendaMock: vi.fn().mockResolvedValue([]),
}));
vi.mock('@/lib/day-agenda', () => ({
  getDayAgenda: getDayAgendaMock,
  readCachedAgenda: vi.fn().mockReturnValue([]),
  cacheAgenda: vi.fn(),
  dismissCalendarEvent: vi.fn(),
  undismissCalendarEvent: vi.fn(),
  dismissKeysFor: vi.fn(() => ['k']),
  setItemDismissed: vi.fn((items: unknown[]) => items),
}));

const { isAnyCalendarConnectedMock } = vi.hoisted(() => ({
  isAnyCalendarConnectedMock: vi.fn(),
}));
vi.mock('@/lib/calendar', () => ({
  getCalendarAccessStatus: vi.fn().mockResolvedValue('notDetermined'),
  isAnyCalendarConnected: isAnyCalendarConnectedMock,
}));

vi.mock('@/lib/safe-listen', () => ({ safeListen: vi.fn(() => () => {}) }));

import { useDayAgenda } from '@/hooks/useDayAgenda';

beforeEach(() => {
  searchParamsGetMock.mockReset().mockReturnValue(null);
  isAnyCalendarConnectedMock.mockReset();
  getDayAgendaMock.mockReset().mockResolvedValue([]);
  try {
    window.localStorage.clear();
  } catch {
    /* not expected in jsdom, but guard anyway */
  }
});

/** Minimal `DayAgendaItem`, matching `lib/day-agenda.test.ts`'s helper shape. */
function agendaItem(partial: Partial<DayAgendaItemLike>): DayAgendaItemLike {
  return {
    id: 'x',
    title: 'Standup',
    startTime: '2026-09-20T15:00:00Z',
    endTime: null,
    source: 'calendar',
    zoomUrl: null,
    attendees: [],
    attendeeCount: 0,
    meetingId: null,
    status: { recorded: false, transcribed: false, summarized: false, speakersIdentified: false },
    dismissed: false,
    ...partial,
  };
}

interface DayAgendaItemLike {
  id: string;
  title: string;
  startTime: string;
  endTime: string | null;
  source: 'calendar' | 'recording' | 'manual';
  zoomUrl: string | null;
  attendees: unknown[];
  attendeeCount: number;
  meetingId: string | null;
  status: { recorded: boolean; transcribed: boolean; summarized: boolean; speakersIdentified: boolean };
  dismissed: boolean;
}

describe('useDayAgenda view mode default (specs/0069 W4)', () => {
  it('defaults to List when no calendar is connected', async () => {
    isAnyCalendarConnectedMock.mockResolvedValue(false);
    const { result } = renderHook(() => useDayAgenda());
    await waitFor(() => expect(result.current.viewMode).toBe('list'));
  });

  it('defaults to Day when one is', async () => {
    isAnyCalendarConnectedMock.mockResolvedValue(true);
    const { result } = renderHook(() => useDayAgenda());
    await waitFor(() => expect(result.current.calendarConnected).toBe(true));
    expect(result.current.viewMode).toBe('day');
  });

  it('an explicit choice wins over the default', async () => {
    isAnyCalendarConnectedMock.mockResolvedValue(false);
    window.localStorage.setItem('nixon.today.viewMode', 'day');
    const { result } = renderHook(() => useDayAgenda());
    // Let the (losing) async connectivity check resolve fully before asserting, so a
    // passing test can't be an accident of timing.
    await waitFor(() => expect(result.current.calendarConnected).toBe(false));
    expect(result.current.viewMode).toBe('day');
  });

  it('reads an explicit ?view=list from the URL', async () => {
    isAnyCalendarConnectedMock.mockResolvedValue(true);
    searchParamsGetMock.mockImplementation((key: string) => (key === 'view' ? 'list' : null));
    const { result } = renderHook(() => useDayAgenda());
    expect(result.current.viewMode).toBe('list');
    await waitFor(() => expect(result.current.calendarConnected).toBe(true));
    // Connected resolving true must not flip an explicit ?view=list back to Day.
    expect(result.current.viewMode).toBe('list');
  });
});

describe('useDayAgenda anti-flicker merge (specs/0069b review fix)', () => {
  // Failure this pins: cache holds calendar rows; you add a manual meeting; a LATER
  // refresh (60s interval, focus, recording-stopped, etc.) comes back with zero
  // calendar items — the anti-flicker fallback used to rebuild the day from cached
  // calendar rows + fresh recordings only, silently dropping the manual row (and its
  // Record/Edit/Delete actions) even though it's DB-backed and was right there in the
  // fresh read.
  it('keeps a manually added meeting when a later refresh returns no calendar items', async () => {
    isAnyCalendarConnectedMock.mockResolvedValue(true);
    const calendarItem = agendaItem({ id: 'cal-1', source: 'calendar' });
    const manualItem = agendaItem({
      id: 'man-1',
      source: 'manual',
      title: 'Call with Sam',
      meetingId: 'meeting-1',
    });

    getDayAgendaMock.mockResolvedValueOnce([calendarItem]);
    const { result } = renderHook(() => useDayAgenda());
    await waitFor(() => expect(result.current.items).toEqual([calendarItem]));

    // The calendar side goes transiently empty, but the manual row is still in the
    // fresh read (it isn't sourced from EventKit at all).
    getDayAgendaMock.mockResolvedValueOnce([manualItem]);
    await act(async () => {
      await result.current.refresh();
    });

    await waitFor(() => {
      expect(result.current.items.some((it) => it.id === 'man-1')).toBe(true);
    });
    // The calendar row is still kept too (the anti-flicker protection it already had).
    expect(result.current.items.some((it) => it.id === 'cal-1')).toBe(true);
  });
});
