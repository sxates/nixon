import { useCallback, useEffect, useMemo, useState } from 'react';
import { useRouter, useSearchParams } from 'next/navigation';
import { toast } from 'sonner';
import { safeListen } from '@/lib/safe-listen';
import {
  getDayAgenda,
  readCachedAgenda,
  cacheAgenda,
  dismissCalendarEvent,
  undismissCalendarEvent,
  dismissKeysFor,
  setItemDismissed,
  type DayAgendaItem,
} from '@/lib/day-agenda';
import { getCalendarAccessStatus, type CalendarAccessStatus } from '@/lib/calendar';
import {
  localDateKey,
  isTodayKey,
  shiftDateKey,
  weekDaysFor,
} from '@/lib/today-timeline';

const REFRESH_INTERVAL_MS = 60 * 1000; // pick up calendar changes

/** `YYYY-MM-DD` local-day key, as written to the `?day=` param. */
function isValidDateKey(v: string | null): v is string {
  return !!v && /^\d{4}-\d{2}-\d{2}$/.test(v);
}

export type AgendaViewMode = 'day' | 'week';

/**
 * Data + navigation state for the Home/Today agenda (specs/0036 WS7, 0038 WS4).
 *
 * Owns: the day/week agenda items (with the today-scoped anti-flicker cache), calendar
 * access status, the ticking `now` clock, the viewed day + day/week mode (mirrored into
 * the `?day=&view=` URL params), and the hide/un-hide (dismiss) mutations.
 *
 * Must be rendered under a Suspense boundary (`useSearchParams`).
 */
export function useDayAgenda() {
  const router = useRouter();
  // The viewed day + mode live in the URL (`?day=&view=`) so returning to Home (e.g. the
  // back button out of a meeting) restores the SAME day you navigated from, not today
  // (specs/0038 feedback). Read once for the initial state; kept in sync via router.replace.
  const searchParams = useSearchParams();

  const [items, setItems] = useState<DayAgendaItem[]>(() => readCachedAgenda());
  const [calendarStatus, setCalendarStatus] = useState<CalendarAccessStatus | null>(null);
  const [loaded, setLoaded] = useState(false);
  // Advances each minute so the now-line moves and phases (upcoming→now→past) re-classify.
  const [now, setNow] = useState<Date>(() => new Date());

  // Date navigation (specs/0038 WS4): which local day the agenda shows, and whether
  // we're in single-day or week mode. Both feed the same `api_get_day_agenda(date)`.
  const [viewDate, setViewDate] = useState<string>(() => {
    const d = searchParams.get('day');
    return isValidDateKey(d) ? d : localDateKey();
  });
  const [viewMode, setViewMode] = useState<AgendaViewMode>(() =>
    searchParams.get('view') === 'week' ? 'week' : 'day',
  );
  // Week mode: one agenda array per day, index-aligned to `weekDaysFor(viewDate)`.
  const [weekItems, setWeekItems] = useState<DayAgendaItem[][]>([]);
  const [weekLoaded, setWeekLoaded] = useState(false);

  const viewIsToday = isTodayKey(viewDate, now);
  const weekDays = useMemo(() => weekDaysFor(viewDate), [viewDate]);

  // Load the agenda for the selected day (or week). For TODAY's single-day view we keep
  // the anti-flicker cache fallback the home count loader used (specs/0024 WS5.1): a
  // cold/failed read that loses all calendar items falls back to the last-good cache;
  // a calendar-bearing read refreshes the cache. Navigated days (specs/0038 WS4) fetch
  // straight — the last-good cache is today-scoped, so we never merge another day into it.
  const refresh = useCallback(async () => {
    const status = await getCalendarAccessStatus();
    setCalendarStatus((prev) => (prev === 'authorized' && status !== 'authorized' ? prev : status));

    if (viewMode === 'week') {
      const agendas = await Promise.all(
        weekDays.map((d) => getDayAgenda(isTodayKey(d) ? undefined : d)),
      );
      setWeekItems(agendas);
      setWeekLoaded(true);
      return;
    }

    // `undefined` for today preserves the original None=>today call; else pass the key.
    const agenda = await getDayAgenda(viewIsToday ? undefined : viewDate);
    if (!viewIsToday) {
      setItems(agenda);
      setLoaded(true);
      return;
    }
    setItems((prev) => {
      const freshHasCalendar = agenda.some((it) => it.source === 'calendar');
      if (freshHasCalendar) {
        cacheAgenda(agenda);
        return agenda;
      }
      // No calendar items this read — keep prior calendar rows, refresh recordings.
      const cached = prev.length ? prev : readCachedAgenda();
      const keptCalendar = cached.filter((it) => it.source === 'calendar');
      if (keptCalendar.length === 0) return agenda; // genuinely nothing but recordings
      const freshRecordings = agenda.filter((it) => it.source === 'recording');
      return [...keptCalendar, ...freshRecordings];
    });
    setLoaded(true);
  }, [viewMode, viewDate, viewIsToday, weekDays]);

  useEffect(() => {
    void refresh();
    const onFocus = () => void refresh();
    window.addEventListener('focus', onFocus);
    const interval = window.setInterval(() => void refresh(), REFRESH_INTERVAL_MS);
    // Move the now-line / re-classify phases roughly every minute.
    const clock = window.setInterval(() => setNow(new Date()), 60 * 1000);
    const disposeStopped = safeListen('recording-stopped', () => void refresh());
    const disposeDiarized = safeListen('diarization-complete', () => void refresh());
    // A background prep pass may have changed a brief (specs/0036) — cheap re-read.
    const disposePrep = safeListen('prep-briefs-updated', () => void refresh());
    return () => {
      window.removeEventListener('focus', onFocus);
      window.clearInterval(interval);
      window.clearInterval(clock);
      disposeStopped();
      disposeDiarized();
      disposePrep();
    };
  }, [refresh]);

  // Jump to a specific local day (specs/0038 WS4). Clear the current view so the loader
  // shows while the new day loads, rather than briefly showing the previous day's rows.
  const goToDate = useCallback((key: string) => {
    setViewDate(key);
    setItems([]);
    setLoaded(false);
    setWeekLoaded(false);
  }, []);

  const step = viewMode === 'week' ? 7 : 1;
  const goPrev = useCallback(() => goToDate(shiftDateKey(viewDate, -step)), [goToDate, viewDate, step]);
  const goNext = useCallback(() => goToDate(shiftDateKey(viewDate, step)), [goToDate, viewDate, step]);
  const goToday = useCallback(() => goToDate(localDateKey()), [goToDate]);

  // Mirror the viewed day/mode into the URL (`?day=&view=`) so leaving and returning to
  // Home restores it — the back button out of a meeting lands on the SAME day you were on,
  // not today (specs/0038 feedback). `replace` keeps this off the history stack: each day
  // change updates the current `/` entry in place rather than piling up entries.
  useEffect(() => {
    const params = new URLSearchParams();
    if (viewDate !== localDateKey()) params.set('day', viewDate);
    if (viewMode === 'week') params.set('view', 'week');
    const qs = params.toString();
    router.replace(qs ? `/?${qs}` : '/', { scroll: false });
  }, [viewDate, viewMode, router]);

  const switchMode = useCallback((mode: AgendaViewMode) => {
    setViewMode(mode);
    setLoaded(false);
    setWeekLoaded(false);
  }, []);

  // Week → day drill-in: open a specific day in the single-day timeline.
  const openDay = useCallback(
    (key: string) => {
      setViewMode('day');
      goToDate(key);
    },
    [goToDate],
  );

  const visibleItems = useMemo(() => items.filter((it) => !it.dismissed), [items]);

  // Hide / un-hide a calendar event from the timeline (specs/0026 dismiss commands,
  // surfaced here per specs/0041 WS5). Optimistic: flip the flag locally so the bubble
  // (dis)appears immediately, revert on failure; an undo toast mirrors the old
  // DayAgenda behavior. A background refresh reconciles with the backend.
  const setDismissed = useCallback((id: string, dismissed: boolean) => {
    setItems((prev) => setItemDismissed(prev, id, dismissed));
  }, []);

  const handleUnhide = useCallback(
    async (item: DayAgendaItem) => {
      setDismissed(item.id, false); // optimistic
      try {
        // Clear BOTH the stable and the legacy key — the dismissal may be stored
        // under either (specs/0029 WS6.2 dual-key migration).
        await undismissCalendarEvent(dismissKeysFor(item));
        void refresh();
      } catch (err) {
        setDismissed(item.id, true); // revert
        toast.error('Could not unhide event', {
          description: err instanceof Error ? err.message : String(err),
        });
      }
    },
    [setDismissed, refresh],
  );

  const handleHide = useCallback(
    async (item: DayAgendaItem) => {
      setDismissed(item.id, true); // optimistic — the bubble disappears immediately
      try {
        // Write only the preferred (sync-stable) key; reads accept old + new.
        await dismissCalendarEvent(dismissKeysFor(item)[0]);
        toast('Event hidden', {
          description: item.title?.trim() || 'Untitled meeting',
          action: { label: 'Undo', onClick: () => void handleUnhide(item) },
        });
        void refresh();
      } catch (err) {
        setDismissed(item.id, false); // revert
        toast.error('Could not hide event', {
          description: err instanceof Error ? err.message : String(err),
        });
      }
    },
    [setDismissed, refresh, handleUnhide],
  );

  return {
    items,
    visibleItems,
    calendarStatus,
    loaded,
    now,
    viewDate,
    viewMode,
    viewIsToday,
    weekDays,
    weekItems,
    weekLoaded,
    refresh,
    goToDate,
    goPrev,
    goNext,
    goToday,
    switchMode,
    openDay,
    handleHide,
    handleUnhide,
  };
}
