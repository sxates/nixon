import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
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
import {
  getCalendarAccessStatus,
  isAnyCalendarConnected,
  type CalendarAccessStatus,
} from '@/lib/calendar';
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

export type AgendaViewMode = 'day' | 'week' | 'list';

const VIEW_MODE_STORAGE_KEY = 'nixon.today.viewMode';

function isAgendaViewMode(v: string | null): v is AgendaViewMode {
  return v === 'day' || v === 'week' || v === 'list';
}

/** The persisted view-mode preference, or null if there isn't one (or storage throws —
 *  private windows throw on `localStorage` access, and Today must still render). */
function readStoredViewMode(): AgendaViewMode | null {
  try {
    const v = localStorage.getItem(VIEW_MODE_STORAGE_KEY);
    return isAgendaViewMode(v) ? v : null;
  } catch {
    return null;
  }
}

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
  // Whether ANY calendar source is connected (EventKit or Google, specs/0069 W4) — null
  // until the one-shot check below resolves. Distinct from `calendarStatus`, which is
  // EventKit-only and drives the (separate) "Connect your calendar" nudge gate.
  const [calendarConnected, setCalendarConnected] = useState<boolean | null>(null);
  const [loaded, setLoaded] = useState(false);
  // Advances each minute so the now-line moves and phases (upcoming→now→past) re-classify.
  const [now, setNow] = useState<Date>(() => new Date());

  // Whether the initial view mode came from an explicit source (the `?view=` URL param
  // or a persisted preference) rather than the bare default. Tracked once at
  // initialization — NOT recomputed later — because the no-calendar default below must
  // never override a real choice, including one made only moments ago via `switchMode`.
  const hasExplicitViewPreferenceRef = useRef(false);

  // Date navigation (specs/0038 WS4): which local day the agenda shows, and whether
  // we're in single-day or week mode. Both feed the same `api_get_day_agenda(date)`.
  const [viewDate, setViewDate] = useState<string>(() => {
    const d = searchParams.get('day');
    return isValidDateKey(d) ? d : localDateKey();
  });
  // Precedence: `?view=` -> persisted `nixon.today.viewMode` -> 'day' (the no-calendar
  // 'list' default, below, only applies when NEITHER of the first two fired).
  const [viewMode, setViewMode] = useState<AgendaViewMode>(() => {
    const fromUrl = searchParams.get('view');
    if (isAgendaViewMode(fromUrl)) {
      hasExplicitViewPreferenceRef.current = true;
      return fromUrl;
    }
    const stored = readStoredViewMode();
    if (stored) {
      hasExplicitViewPreferenceRef.current = true;
      return stored;
    }
    return 'day';
  });

  // specs/0069 W4 — a machine with no calendar opens on the list, not on an hour grid
  // with nothing in it. This resolves asynchronously, so a fresh no-calendar profile
  // sees Day for one frame before it settles; an explicit choice never gets overridden
  // at all (`hasExplicitViewPreferenceRef`, set only at initialization above).
  useEffect(() => {
    let cancelled = false;
    void isAnyCalendarConnected().then((connected) => {
      if (cancelled) return;
      setCalendarConnected(connected);
      if (!connected && !hasExplicitViewPreferenceRef.current) {
        setViewMode('list');
      }
    });
    return () => {
      cancelled = true;
    };
    // Runs once — a mid-session calendar connect/disconnect doesn't retroactively
    // change which view you're looking at.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
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
    if (viewMode === 'week' || viewMode === 'list') params.set('view', viewMode);
    const qs = params.toString();
    router.replace(qs ? `/?${qs}` : '/', { scroll: false });
  }, [viewDate, viewMode, router]);

  const switchMode = useCallback((mode: AgendaViewMode) => {
    // A deliberate switch IS an explicit preference — persist it so the no-calendar
    // 'list' default (above) never fights it on a later mount, and so the choice
    // survives a reload. Wrapped in try/catch: private windows throw on `localStorage`.
    hasExplicitViewPreferenceRef.current = true;
    try {
      localStorage.setItem(VIEW_MODE_STORAGE_KEY, mode);
    } catch {
      /* private window — the in-memory ref above still prevents this session's
         auto-switch from overriding the choice just made */
    }
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
    calendarConnected,
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
