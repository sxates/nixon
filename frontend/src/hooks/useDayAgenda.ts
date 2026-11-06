import { useCallback, useEffect, useMemo, useState } from 'react';
import { useSearchParams } from 'next/navigation';
import { toast } from 'sonner';
import { safeListen } from '@/lib/safe-listen';
import {
  getDayAgenda,
  readDayAgenda,
  readCachedAgenda,
  cacheAgenda,
  clearCachedAgenda,
  dismissCalendarEvent,
  undismissCalendarEvent,
  dismissKeysFor,
  setItemDismissed,
  type DayAgendaItem,
} from '@/lib/day-agenda';
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

/** Which agenda the Meetings page is showing; `null` while it shows Month or List. */
export type AgendaMode = 'day' | 'week';

/**
 * Data + navigation state for the Meetings page's Agenda and Week views (specs/0036 WS7,
 * 0038 WS4).
 *
 * Owns: the day/week agenda items (with the today-scoped anti-flicker cache), the ticking
 * `now` clock, the viewed day, and the hide/un-hide (dismiss) mutations. Which view is
 * showing — and mirroring it into the URL — belongs to `useMeetingsView`; `mode` is that
 * view seen from here (`viewMode`), and `null` (Month / List) stops the agenda reads.
 *
 * Must be rendered under a Suspense boundary (`useSearchParams`).
 */
export function useDayAgenda(viewMode: AgendaMode | null = 'day') {
  // The viewed day is restored from `?day=` so returning to Meetings (e.g. the back button
  // out of a meeting) lands on the SAME day you navigated from, not today (specs/0038
  // feedback). Read once for the initial state; `useMeetingsUrlSync` keeps it in the URL.
  const searchParams = useSearchParams();

  // Seeded from the last-good cache for the day being opened (keyed by date, specs/0075
  // W3 — never another day's rows). Only today is ever cached, so other days start empty.
  const [items, setItems] = useState<DayAgendaItem[]>(() => {
    const d = searchParams.get('day');
    return readCachedAgenda(isValidDateKey(d) ? d : localDateKey());
  });
  const [loaded, setLoaded] = useState(false);
  // Advances each minute so the now-line moves and phases (upcoming→now→past) re-classify.
  const [now, setNow] = useState<Date>(() => new Date());

  // Date navigation (specs/0038 WS4): which local day the agenda shows, and whether
  // we're in single-day or week mode. Both feed the same `api_get_day_agenda(date)`.
  const [viewDate, setViewDate] = useState<string>(() => {
    const d = searchParams.get('day');
    return isValidDateKey(d) ? d : localDateKey();
  });
  // Week mode: one agenda array per day, index-aligned to `weekDaysFor(viewDate)`.
  const [weekItems, setWeekItems] = useState<DayAgendaItem[][]>([]);
  const [weekLoaded, setWeekLoaded] = useState(false);

  const viewIsToday = isTodayKey(viewDate, now);
  const weekDays = useMemo(() => weekDaysFor(viewDate), [viewDate]);

  // Load the agenda for the selected day (or week). For TODAY's single-day view we keep
  // the anti-flicker cache fallback the home count loader used (specs/0024 WS5.1): a
  // cold/failed EventKit read that loses all calendar items falls back to the last-good
  // cache; a calendar-bearing read refreshes the cache. Google (specs/0075 W3) reads a
  // local cache and has no cold-read failure, so an empty Google read is the truth — a
  // meeting rescheduled off today leaves today. Navigated days (specs/0038 WS4) fetch
  // straight — the last-good cache is today-scoped, so we never merge another day into it.
  const refresh = useCallback(async () => {
    if (viewMode === null) return;
    if (viewMode === 'week') {
      const agendas = await Promise.all(
        weekDays.map((d) => getDayAgenda(isTodayKey(d) ? undefined : d)),
      );
      setWeekItems(agendas);
      setWeekLoaded(true);
      return;
    }

    // `undefined` for today preserves the original None=>today call; else pass the key.
    const { items: agenda, calendarSource } = await readDayAgenda(
      viewIsToday ? undefined : viewDate,
    );
    if (!viewIsToday) {
      setItems(agenda);
      setLoaded(true);
      return;
    }
    setItems((prev) => {
      const freshHasCalendar = agenda.some((it) => it.source === 'calendar');
      if (freshHasCalendar) {
        cacheAgenda(viewDate, agenda);
        return agenda;
      }
      if (calendarSource === 'google') {
        clearCachedAgenda(viewDate);
        return agenda;
      }
      // No calendar items this read — keep prior calendar rows, refresh everything else.
      // Manual entries (specs/0069 W3) come from our own DB, not EventKit, so — like
      // recordings — they're reliable even on a read where the calendar side glitched;
      // taking them from `agenda` (not the cache) also means an edit/delete of a manual
      // row shows up immediately instead of showing stale cached content. Only calendar
      // rows need the cache fallback, since only EventKit has this transient-empty mode.
      const cached = prev.length ? prev : readCachedAgenda(viewDate);
      const keptCalendar = cached.filter((it) => it.source === 'calendar');
      if (keptCalendar.length === 0) return agenda; // genuinely nothing but recordings/manual
      const freshOther = agenda.filter((it) => it.source === 'recording' || it.source === 'manual');
      return [...keptCalendar, ...freshOther];
    });
    setLoaded(true);
  }, [viewMode, viewDate, viewIsToday, weekDays]);

  // A switch between Day and Week (or into either) shows the loader until its first read,
  // rather than the other view's rows. Declared before the refresh effect so it runs first.
  useEffect(() => {
    setLoaded(false);
    setWeekLoaded(false);
  }, [viewMode]);

  useEffect(() => {
    // Month and List have their own data; no agenda reads or timers while they show.
    if (viewMode === null) return;
    // The clock below doesn't tick while Month/List show, so catch it up on the way back.
    setNow(new Date());
    void refresh();
    const onFocus = () => void refresh();
    window.addEventListener('focus', onFocus);
    const interval = window.setInterval(() => void refresh(), REFRESH_INTERVAL_MS);
    // Move the now-line / re-classify phases roughly every minute.
    const clock = window.setInterval(() => setNow(new Date()), 60 * 1000);
    // A recording started without an invite has no row until the agenda re-reads — and no
    // row means no recording marker (owner report 2026-09-23).
    const disposeStarted = safeListen('recording-started', () => void refresh());
    const disposeStopped = safeListen('recording-stopped', () => void refresh());
    const disposeDiarized = safeListen('diarization-complete', () => void refresh());
    // A background prep pass may have changed a brief (specs/0036) — cheap re-read.
    const disposePrep = safeListen('prep-briefs-updated', () => void refresh());
    // A Google sync pass finished (specs/0075 W1) — the local cache may have moved rows.
    const disposeGoogle = safeListen('google-calendar-synced', () => void refresh());
    return () => {
      window.removeEventListener('focus', onFocus);
      window.clearInterval(interval);
      window.clearInterval(clock);
      disposeStarted();
      disposeStopped();
      disposeDiarized();
      disposePrep();
      disposeGoogle();
    };
  }, [refresh, viewMode]);

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
    handleHide,
    handleUnhide,
  };
}
