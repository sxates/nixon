/**
 * The Agenda / Week views' date labels: the one beside the nav arrows, and the summary
 * line under the Meetings title. Pure — the page passes in what it shows.
 */

import type { DayAgendaItem } from '@/lib/day-agenda';
import {
  dayCounts,
  dayLabel,
  localDateKey,
  parseLocalDateKey,
  type TimelineContext,
} from '@/lib/today-timeline';

export interface AgendaLabelInput {
  mode: 'day' | 'week';
  now: Date;
  viewDate: string;
  weekDays: string[];
  visibleItems: DayAgendaItem[];
  weekItems: DayAgendaItem[][];
  ctx: TimelineContext;
}

function shortDate(key: string): string {
  return parseLocalDateKey(key).toLocaleDateString([], { month: 'short', day: 'numeric' });
}

/** "Jul 6 – Jul 12". */
export function weekRangeLabel(weekDays: string[]): string {
  return `${shortDate(weekDays[0])} – ${shortDate(weekDays[6])}`;
}

/** Beside the nav arrows: "Today" / "Tomorrow" / a date, or "This week" / a range. */
export function agendaNavLabel({ mode, now, viewDate, weekDays }: AgendaLabelInput): string {
  if (mode === 'week') {
    return weekDays.includes(localDateKey(now)) ? 'This week' : weekRangeLabel(weekDays);
  }
  return dayLabel(viewDate, now);
}

/** Under the title: the day (or week) in view and what's in it. */
export function agendaSummary(input: AgendaLabelInput): string {
  const { mode, now, viewDate, weekDays, visibleItems, weekItems, ctx } = input;
  if (mode === 'week') {
    const total = weekItems.reduce((n, arr) => n + arr.filter((it) => !it.dismissed).length, 0);
    const range = weekDays.includes(localDateKey(now))
      ? `Week of ${parseLocalDateKey(weekDays[0]).toLocaleDateString([], { month: 'long', day: 'numeric' })}`
      : weekRangeLabel(weekDays);
    return total > 0 ? `${range} · ${total} meeting${total === 1 ? '' : 's'}` : range;
  }
  const parts = [
    parseLocalDateKey(viewDate).toLocaleDateString([], {
      weekday: 'long',
      month: 'long',
      day: 'numeric',
    }),
  ];
  // Counted off the chips' own state, so a Processing row isn't also called "recorded".
  const { total, recorded, processing } = dayCounts(visibleItems, ctx);
  if (total > 0) parts.push(`${total} meeting${total === 1 ? '' : 's'}`);
  if (recorded > 0) parts.push(`${recorded} recorded`);
  if (processing > 0) parts.push(`${processing} processing`);
  return parts.join(' · ');
}
