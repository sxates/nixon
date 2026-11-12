'use client';

/**
 * The Meetings page's Month view (specs/0054 W3).
 *
 * The flat chronological list stops being findable once a few months of
 * recordings accumulate ("meeting volume has reached hundreds, back a couple of
 * months"), so this offers the other way people remember a meeting: roughly when
 * it happened.
 *
 * With a calendar connected (`events`) it also shows the calendar's events — the
 * ones coming up, and past ones that were never recorded — read and filtered as
 * the Week view reads them (`useMonthEvents`, `monthEventItems`), drawn in the
 * agenda's own per-state chrome, and opened the way Week opens them. Without a
 * calendar it stays recorded-only.
 *
 * Loads one bounded range per displayed month via `api_get_meetings_in_range`
 * rather than the whole table. All date math lives in `@/lib/month-grid`.
 *
 * Sizes to its container rather than to its content: the grid rows divide whatever
 * vertical space the page gives it (`auto-rows-fr`), so a taller window means taller
 * day cells instead of dead space below a fixed-height grid.
 */

import { useCallback, useEffect, useMemo, useState } from 'react';
import { NavSlotPortal, useHasNavSlot } from '@/components/Meetings/NavSlot';
import { invoke } from '@tauri-apps/api/core';
import { Callout } from '@/components/ui/callout';
import { DateNavigator } from '@/components/ui/date-navigator';
import { ListRow } from '@/components/ui/list-row';
import { Spinner } from '@/components/ui/spinner';
import { cn } from '@/lib/utils';
import type { DayAgendaItem } from '@/lib/day-agenda';
import { monthEventItems } from '@/lib/month-events';
import { itemVisualState, type TimelineContext } from '@/lib/today-timeline';
import { blockClasses, StateChip } from '@/components/Today/TimelineBlock';
import { useMonthEvents } from '@/hooks/useMonthEvents';
import {
  bucketByLocalDay,
  buildMonthGrid,
  gridRangeUtc,
  localDayKey,
  monthLabel,
  shiftMonth,
  weekdayLabels,
} from '@/lib/month-grid';

/** Shape returned by `api_get_meetings_in_range` (Rust `MonthMeeting`). */
export interface MonthMeeting {
  id: string;
  title: string;
  createdAt: string;
  durationSeconds?: number;
  hasSummary: boolean;
}

/**
 * Chips rendered inline per day. The row floor below is sized so this many always
 * fit whole — a chip clipped mid-glyph reads as a rendering bug. Days with more
 * than this are honest about it via the header count badge.
 */
const MAX_CHIPS_PER_DAY = 3;

/** One thing on a day: a recording, or a calendar event that has none. */
type DayEntry =
  | { kind: 'recording'; id: string; title: string; start: string; meeting: MonthMeeting }
  | { kind: 'event'; id: string; title: string; start: string; item: DayAgendaItem };

/** Calendar events in the Month view: what they need from the page (Week's own wiring). */
export interface MonthCalendarEvents {
  /** For each event's state (upcoming / now / missed), as the agenda chips read it. */
  ctx: TimelineContext;
  /** Clicking an event does what it does in Week view (`useAgendaActions.handleSelect`). */
  onSelect: (item: DayAgendaItem) => void;
}

function entryTime(start: string): string {
  return new Date(start).toLocaleTimeString(undefined, { hour: 'numeric', minute: '2-digit' });
}

export function MonthCalendar({
  onOpenMeeting,
  activeRecordingMeetingId,
  events,
}: {
  onOpenMeeting: (id: string) => void;
  activeRecordingMeetingId?: string | null;
  /** Set with a calendar connected: the month also shows its events. */
  events?: MonthCalendarEvents;
}) {
  const hasSlot = useHasNavSlot();
  const today = useMemo(() => new Date(), []);
  const [cursor, setCursor] = useState(() => ({
    year: today.getFullYear(),
    monthIndex: today.getMonth(),
  }));
  const [meetings, setMeetings] = useState<MonthMeeting[]>([]);
  const [isLoading, setIsLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [selectedDay, setSelectedDay] = useState<string | null>(null);

  const { year, monthIndex } = cursor;

  useEffect(() => {
    let cancelled = false;
    const load = async () => {
      setIsLoading(true);
      setError(null);
      try {
        const { start, end } = gridRangeUtc(year, monthIndex);
        const rows = await invoke<MonthMeeting[]>('api_get_meetings_in_range', { start, end });
        if (!cancelled) setMeetings(Array.isArray(rows) ? rows : []);
      } catch (err) {
        console.error('Failed to load meetings for the month:', err);
        if (!cancelled) setError('Could not load this month’s meetings.');
      } finally {
        if (!cancelled) setIsLoading(false);
      }
    };
    void load();
    return () => {
      cancelled = true;
    };
  }, [year, monthIndex]);

  const agendas = useMonthEvents(year, monthIndex, !!events);

  const grid = useMemo(() => buildMonthGrid(year, monthIndex), [year, monthIndex]);
  const byDay = useMemo(() => {
    const recordedIds = new Set(meetings.map((m) => m.id));
    const entries: DayEntry[] = [
      ...meetings.map(
        (m): DayEntry => ({ kind: 'recording', id: m.id, title: m.title, start: m.createdAt, meeting: m }),
      ),
      ...monthEventItems(agendas, recordedIds).map(
        (item): DayEntry => ({
          kind: 'event',
          id: `${item.id}|${item.startTime}`,
          title: item.title?.trim() || 'Untitled meeting',
          start: item.startTime,
          item,
        }),
      ),
    ];
    const buckets = bucketByLocalDay(entries, (e) => e.start);
    for (const list of buckets.values()) list.sort((a, b) => a.start.localeCompare(b.start));
    return buckets;
  }, [meetings, agendas]);

  const openEntry = (e: DayEntry) =>
    e.kind === 'recording' ? onOpenMeeting(e.id) : events?.onSelect(e.item);
  /** Recordings keep their summary tint; events wear the agenda's per-state chrome. */
  const chipClass = (e: DayEntry) => {
    if (e.kind === 'event') {
      return events ? cn('border', blockClasses(itemVisualState(e.item, events.ctx))) : '';
    }
    // The live recording wears the REC lamp — red is recording only, never `destructive`.
    if (e.id === activeRecordingMeetingId) return 'bg-record text-record-foreground';
    return e.meeting.hasSummary ? 'bg-chart-1/15 text-foreground' : 'bg-muted text-muted-foreground';
  };
  const todayKey = localDayKey(today);

  const step = useCallback((delta: number) => {
    setSelectedDay(null);
    setCursor((c) => shiftMonth(c.year, c.monthIndex, delta));
  }, []);

  const goToday = useCallback(() => {
    setSelectedDay(null);
    const now = new Date();
    setCursor({ year: now.getFullYear(), monthIndex: now.getMonth() });
  }, []);

  const selected = selectedDay ? (byDay.get(selectedDay) ?? []) : null;

  // The spinner and Today sit LEFT of the arrows, so neither moves them (the nav is
  // right-aligned in the view-tab row).
  const nav = (
    <DateNavigator
      label={monthLabel(year, monthIndex)}
      labelClassName="min-w-[9.5rem]"
      onPrev={() => step(-1)}
      onNext={() => step(1)}
      prevLabel="Previous month"
      nextLabel="Next month"
      onToday={goToday}
      leading={isLoading ? <Spinner size="sm" className="text-muted-foreground" /> : null}
    />
  );

  return (
    <div className="flex h-full min-h-0 flex-col">
      {/* On the Meetings page the month navigation sits at the right of the view-tab row
          (NavSlot); standalone it keeps its own row. */}
      {hasSlot ? (
        <NavSlotPortal>{nav}</NavSlotPortal>
      ) : (
        <div className="mb-3 flex flex-shrink-0 items-center justify-end gap-3">{nav}</div>
      )}

      {error ? (
        <Callout tone="error">{error}</Callout>
      ) : (
        <>
          <div className="grid flex-shrink-0 grid-cols-7 gap-px">
            {weekdayLabels().map((label) => (
              <div
                key={label}
                className="u-section-label pb-1 text-center"
                aria-hidden="true"
              >
                {label}
              </div>
            ))}
          </div>

          {/* `1fr` rows over `flex-1 min-h-0` is what makes a taller window mean taller
              day cells rather than dead space below a fixed-height grid. The `minmax`
              floor keeps MAX_CHIPS_PER_DAY chips whole; when the window is too short to
              honour it the grid scrolls, which beats slicing a chip in half. */}
          <div
            className="grid min-h-0 flex-1 auto-rows-[minmax(6.5rem,1fr)] grid-cols-7 gap-px overflow-y-auto rounded-lg border border-border bg-border"
            role="grid"
            aria-label={`Meetings in ${monthLabel(year, monthIndex)}`}
          >
            {grid.map((cell) => {
              const key = localDayKey(cell.date);
              const dayMeetings = byDay.get(key) ?? [];
              const isToday = key === todayKey;
              const isSelected = key === selectedDay;
              return (
                <button // ui-gate: allow a month-grid day is a `gridcell` with a top-aligned layout; no primitive is a grid cell
                  key={key}
                  type="button"
                  role="gridcell"
                  aria-label={`${cell.date.toDateString()}, ${dayMeetings.length} meeting${
                    dayMeetings.length === 1 ? '' : 's'
                  }`}
                  aria-selected={isSelected}
                  // Empty days are selectable too — clicking one should feel like it
                  // did something, and the detail pane says the day was free.
                  onClick={() => setSelectedDay(key)}
                  className={cn(
                    // Buttons centre their content by default, which floats a lone
                    // day number to the middle of a tall cell. Force a top-aligned
                    // column instead.
                    'flex min-h-0 flex-col items-stretch justify-start overflow-hidden bg-background p-1.5 text-left',
                    'transition-colors hover:bg-accent/60',
                    'focus:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring',
                    !cell.inMonth && 'bg-muted/30',
                    isSelected && 'bg-accent',
                  )}
                >
                  <span className="flex flex-shrink-0 items-center justify-between gap-1">
                    <span
                      className={cn(
                        'inline-flex h-5 min-w-5 items-center justify-center rounded-sm px-1 text-caption tabular-nums',
                        cell.inMonth ? 'text-foreground' : 'text-muted-foreground/60',
                        // Today is the amber lamp, as everywhere else.
                        isToday && 'border border-brand-edge bg-brand-wash font-semibold text-brand',
                      )}
                    >
                      {cell.date.getDate()}
                    </span>
                    {/* The day's real total. Chips below clip to whatever the cell
                        height allows, so this badge — not the chip list — is what
                        tells the truth about how busy the day was. */}
                    {dayMeetings.length > 1 && (
                      <span className="pr-0.5 text-label tabular-nums text-muted-foreground">
                        {dayMeetings.length}
                      </span>
                    )}
                  </span>

                  <span className="mt-1 block min-h-0 flex-1 space-y-0.5 overflow-hidden">
                    {dayMeetings.slice(0, MAX_CHIPS_PER_DAY).map((m) => (
                      <span
                        key={m.id}
                        title={m.title}
                        data-entry={m.kind}
                        className={cn(
                          // Sentence case at 11px: these are meeting titles, not engraved labels.
                          'block truncate rounded-sm px-1 py-0.5 text-caption font-semibold leading-tight',
                          chipClass(m),
                        )}
                      >
                        {m.title}
                      </span>
                    ))}
                  </span>
                </button>
              );
            })}
          </div>
        </>
      )}

      {selected && (
        // Capped so selecting a busy day cannot squeeze the grid away; scrolls instead.
        <div className="mt-3 max-h-[32%] flex-shrink-0 overflow-y-auto">
          <h3 className="u-section-label mb-1 px-1">
            {new Date(`${selectedDay}T12:00:00`).toLocaleDateString(undefined, {
              weekday: 'long',
              month: 'long',
              day: 'numeric',
            })}
          </h3>
          {selected.length === 0 ? (
            <p className="u-meta px-1 py-2">
              {events ? 'No meetings this day.' : 'No meetings recorded this day.'}
            </p>
          ) : (
            <div className="flex flex-col gap-0.5">
              {selected.map((m) => {
                const state = m.kind === 'event' && events ? itemVisualState(m.item, events.ctx) : null;
                return (
                  <ListRow
                    key={m.id}
                    onClick={() => openEntry(m)}
                    title={
                      <span className={state === 'past-unrecorded' ? 'text-muted-foreground' : undefined}>
                        {m.title}
                      </span>
                    }
                    trailing={
                      <>
                        {state && <StateChip state={state} />}
                        <span className="u-row-meta tabular-nums">{entryTime(m.start)}</span>
                      </>
                    }
                  />
                );
              })}
            </div>
          )}
        </div>
      )}

      {!isLoading && !error && !selected && byDay.size === 0 && (
        <p className="flex-shrink-0 py-4 text-center text-sm text-muted-foreground">
          {events ? 'No meetings' : 'No recordings'} in {monthLabel(year, monthIndex)}.
        </p>
      )}
    </div>
  );
}
