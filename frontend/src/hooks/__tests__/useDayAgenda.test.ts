import { describe, it, expect, vi, beforeEach } from 'vitest';
import { act, renderHook, waitFor } from '@testing-library/react';

// The Meetings page's Agenda / Week data. Which view shows (and its persistence) moved to
// `useMeetingsView` when Today and All meetings merged; see its own suite.

const { searchParamsGetMock } = vi.hoisted(() => ({
  searchParamsGetMock: vi.fn((_key: string) => null as string | null),
}));
vi.mock('next/navigation', () => ({
  useRouter: () => ({ push: vi.fn(), replace: vi.fn() }),
  useSearchParams: () => ({ get: searchParamsGetMock }),
}));

// `getDayAgendaMock` supplies the items; `sourceRef.current` the calendar source the
// backend reports with them (specs/0075 W3). The date-keyed cache helpers are the real
// ones, over jsdom's sessionStorage (cleared in beforeEach).
const { getDayAgendaMock, readDayAgendaMock, sourceRef } = vi.hoisted(() => {
  const sourceRef = { current: 'eventkit' as 'eventkit' | 'google' | 'unknown' };
  const getDayAgendaMock = vi.fn().mockResolvedValue([]);
  const readDayAgendaMock = vi.fn(async (date?: string) => ({
    items: await getDayAgendaMock(date),
    calendarSource: sourceRef.current,
  }));
  return { getDayAgendaMock, readDayAgendaMock, sourceRef };
});
vi.mock('@/lib/day-agenda', async () => {
  const actual = await vi.importActual<typeof import('@/lib/day-agenda')>('@/lib/day-agenda');
  return {
    ...actual,
    getDayAgenda: getDayAgendaMock,
    readDayAgenda: readDayAgendaMock,
    dismissCalendarEvent: vi.fn(),
    undismissCalendarEvent: vi.fn(),
    dismissKeysFor: vi.fn(() => ['k']),
    setItemDismissed: vi.fn((items: unknown[]) => items),
  };
});

const { isAnyCalendarConnectedMock } = vi.hoisted(() => ({
  isAnyCalendarConnectedMock: vi.fn(),
}));
vi.mock('@/lib/calendar', () => ({
  getCalendarAccessStatus: vi.fn().mockResolvedValue('notDetermined'),
  isAnyCalendarConnected: isAnyCalendarConnectedMock,
}));

// Records each subscribed handler by event name so a test can fire the event.
const { listeners } = vi.hoisted(() => ({ listeners: new Map<string, () => void>() }));
vi.mock('@/lib/safe-listen', () => ({
  safeListen: vi.fn((event: string, handler: () => void) => {
    listeners.set(event, handler);
    return () => {};
  }),
}));

import { useDayAgenda } from '@/hooks/useDayAgenda';
import { cacheAgenda, type DayAgendaItem } from '@/lib/day-agenda';
import { localDateKey, shiftDateKey } from '@/lib/today-timeline';

beforeEach(() => {
  searchParamsGetMock.mockReset().mockReturnValue(null);
  isAnyCalendarConnectedMock.mockReset();
  getDayAgendaMock.mockReset().mockResolvedValue([]);
  readDayAgendaMock.mockClear();
  sourceRef.current = 'eventkit';
  listeners.clear();
  sessionStorage.clear();
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

describe('useDayAgenda mode', () => {
  it('reads nothing while Month or List shows (mode null)', async () => {
    renderHook(() => useDayAgenda(null));
    await act(async () => {});
    expect(readDayAgendaMock).not.toHaveBeenCalled();
    expect(getDayAgendaMock).not.toHaveBeenCalled();
  });

  it('reads the day in Agenda and seven days in Week', async () => {
    const { rerender } = renderHook(({ mode }: { mode: 'day' | 'week' }) => useDayAgenda(mode), {
      initialProps: { mode: 'day' },
    });
    await waitFor(() => expect(readDayAgendaMock).toHaveBeenCalled());
    rerender({ mode: 'week' });
    await waitFor(() => expect(getDayAgendaMock).toHaveBeenCalledTimes(1 + 7));
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

describe('useDayAgenda calendar source + date-keyed cache (specs/0075 W3)', () => {
  const cal = () => agendaItem({ id: 'cal-1', source: 'calendar', title: 'Moved meeting' });

  it('drops calendar rows when a Google read comes back with none (a reschedule off today)', async () => {
    isAnyCalendarConnectedMock.mockResolvedValue(true);
    sourceRef.current = 'google';
    getDayAgendaMock.mockResolvedValueOnce([cal()]);
    const { result } = renderHook(() => useDayAgenda());
    await waitFor(() => expect(result.current.items).toEqual([cal()]));

    getDayAgendaMock.mockResolvedValueOnce([]);
    await act(async () => {
      await result.current.refresh();
    });
    expect(result.current.items).toEqual([]);
    // …and the cache is forgotten too, so the next mount doesn't bring the row back.
    const { result: remount } = renderHook(() => useDayAgenda());
    expect(remount.current.items).toEqual([]);
    await act(async () => {});
  });

  it('keeps the previous calendar rows when an EventKit read comes back with none', async () => {
    isAnyCalendarConnectedMock.mockResolvedValue(true);
    sourceRef.current = 'eventkit';
    getDayAgendaMock.mockResolvedValueOnce([cal()]);
    const { result } = renderHook(() => useDayAgenda());
    await waitFor(() => expect(result.current.items).toEqual([cal()]));

    getDayAgendaMock.mockResolvedValueOnce([]);
    await act(async () => {
      await result.current.refresh();
    });
    expect(result.current.items).toEqual([cal()]);
  });

  it("never seeds today from another day's cached rows", async () => {
    isAnyCalendarConnectedMock.mockResolvedValue(true);
    const yesterday = shiftDateKey(localDateKey(), -1);
    cacheAgenda(yesterday, [cal() as DayAgendaItem]);
    // Hold the first read so we see the cache-seeded initial state, then an empty
    // EventKit read (the anti-flicker path that falls back to the cache).
    let resolve: (v: DayAgendaItemLike[]) => void = () => {};
    getDayAgendaMock.mockReturnValueOnce(new Promise((r) => (resolve = r)));
    const { result } = renderHook(() => useDayAgenda());
    expect(result.current.items).toEqual([]);
    await act(async () => {
      resolve([]);
    });
    await waitFor(() => expect(readDayAgendaMock).toHaveBeenCalled());
    expect(result.current.items).toEqual([]);
  });

  it('re-reads the agenda when a Google sync finishes', async () => {
    isAnyCalendarConnectedMock.mockResolvedValue(true);
    renderHook(() => useDayAgenda());
    await waitFor(() => expect(readDayAgendaMock).toHaveBeenCalled());
    const before = readDayAgendaMock.mock.calls.length;
    const handler = listeners.get('google-calendar-synced');
    expect(handler).toBeDefined();
    await act(async () => {
      handler!();
    });
    expect(readDayAgendaMock.mock.calls.length).toBeGreaterThan(before);
  });
});
