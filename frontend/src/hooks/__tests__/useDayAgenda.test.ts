import { describe, it, expect, vi, beforeEach } from 'vitest';
import { renderHook, waitFor } from '@testing-library/react';

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

vi.mock('@/lib/day-agenda', () => ({
  getDayAgenda: vi.fn().mockResolvedValue([]),
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
  try {
    window.localStorage.clear();
  } catch {
    /* not expected in jsdom, but guard anyway */
  }
});

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
