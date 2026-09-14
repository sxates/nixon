'use client';

import type { DayAgendaItem } from '@/lib/day-agenda';
import { formatClockTime } from '@/lib/calendar';
import {
  itemVisualState,
  localDateKey,
  parseLocalDateKey,
  type TimelineContext,
  type TimelineVisualState,
} from '@/lib/today-timeline';
import { StateChip } from './TimelineBlock';

/** One compact meeting row inside a week-view day card (specs/0038 WS4). */
function WeekRow({
  item,
  state,
  onSelect,
}: {
  item: DayAgendaItem;
  state: TimelineVisualState;
  onSelect: (item: DayAgendaItem) => void;
}) {
  const start = new Date(item.startTime);
  const validStart = !Number.isNaN(start.getTime());
  const title = item.title?.trim() || 'Untitled meeting';
  return (
    <button
      type="button"
      onClick={() => onSelect(item)}
      title={title}
      className="flex w-full items-center gap-3 px-4 py-2 text-left transition-colors hover:bg-accent focus:outline-none focus-visible:ring-2 focus-visible:ring-ring"
    >
      <span className="w-[52px] flex-shrink-0 text-[11px] tabular-nums text-muted-foreground">
        {validStart ? formatClockTime(start) : '--:--'}
      </span>
      <span
        className={`min-w-0 flex-1 truncate text-[13px] font-semibold ${
          state === 'past-unrecorded' ? 'text-muted-foreground' : 'text-foreground'
        }`}
      >
        {title}
      </span>
      <StateChip state={state} />
    </button>
  );
}

/**
 * Minimal week view (specs/0038 WS4): a vertical list of the seven Monday-started
 * days, each a card of compact meeting rows reusing the day view's phase styling
 * (`itemVisualState` / `StateChip`). Clicking a day header drops into its day view;
 * clicking a row routes exactly like the day timeline (`onSelectItem`).
 */
export function WeekView({
  weekDays,
  weekItems,
  ctx,
  now,
  onSelectItem,
  onOpenDay,
}: {
  weekDays: string[];
  weekItems: DayAgendaItem[][];
  ctx: TimelineContext;
  now: Date;
  onSelectItem: (item: DayAgendaItem) => void;
  onOpenDay: (dateKey: string) => void;
}) {
  const todayKey = localDateKey(now);
  return (
    <div className="space-y-2.5 pb-2">
      {weekDays.map((dateKey, i) => {
        const dayItems = (weekItems[i] ?? []).filter((it) => !it.dismissed);
        const d = parseLocalDateKey(dateKey);
        const isDayToday = dateKey === todayKey;
        return (
          <div
            key={dateKey}
            className={`overflow-hidden rounded-[3px] border bg-card ${
              isDayToday ? 'border-brand/45' : 'border-border'
            }`}
          >
            <button
              type="button"
              onClick={() => onOpenDay(dateKey)}
              className="flex w-full items-center justify-between px-4 py-2.5 text-left transition-colors hover:bg-accent focus:outline-none focus-visible:ring-2 focus-visible:ring-ring"
            >
              <span className="flex items-baseline gap-2">
                <span
                  className={`text-[13.5px] font-semibold ${isDayToday ? 'text-brand' : 'text-foreground'}`}
                >
                  {d.toLocaleDateString([], { weekday: 'long' })}
                </span>
                <span className="u-meta">
                  {d.toLocaleDateString([], { month: 'short', day: 'numeric' })}
                </span>
              </span>
              <span className="u-meta">
                {dayItems.length === 0
                  ? 'Free'
                  : `${dayItems.length} meeting${dayItems.length === 1 ? '' : 's'}`}
              </span>
            </button>
            {dayItems.length > 0 && (
              <div className="divide-y divide-border/50 border-t border-border/50">
                {dayItems.map((it) => (
                  <WeekRow
                    key={it.id}
                    item={it}
                    state={itemVisualState(it, ctx)}
                    onSelect={onSelectItem}
                  />
                ))}
              </div>
            )}
          </div>
        );
      })}
    </div>
  );
}
