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
import { ListRow } from '@/components/ui/list-row';
import { DayGroupCard } from '@/components/Meetings/DayGroupCard';
import { StateChip } from './TimelineBlock';
import { AgendaAttendees } from './AgendaAttendees';
import { AgendaRowMenu, type AgendaRowActions } from './AgendaRowMenu';

/** Start-time order for a day's rows; an unparseable start sorts last, ties keep input order. */
export function byStartTime(a: DayAgendaItem, b: DayAgendaItem): number {
  const ta = new Date(a.startTime).getTime();
  const tb = new Date(b.startTime).getTime();
  const na = Number.isNaN(ta);
  const nb = Number.isNaN(tb);
  if (na || nb) return na === nb ? 0 : na ? 1 : -1;
  return ta - tb;
}

/** One compact meeting row inside a week-view day card (specs/0038 WS4). */
function WeekRow({
  item,
  state,
  ctx,
  onSelect,
  actions,
}: {
  item: DayAgendaItem;
  state: TimelineVisualState;
  ctx: TimelineContext;
  onSelect: (item: DayAgendaItem) => void;
  actions: AgendaRowActions;
}) {
  const start = new Date(item.startTime);
  const validStart = !Number.isNaN(start.getTime());
  const title = item.title?.trim() || 'Untitled meeting';
  return (
    // `ListRow` keeps the `…` menu OUTSIDE the row's button — a button inside a button is
    // invalid HTML that browsers silently un-nest (which is how the week row lost its menu
    // in the first place).
    <ListRow
      ruled
      menuReveal="hover"
      onClick={() => onSelect(item)}
      lead={
        <span className="u-row-meta w-14 tabular-nums">
          {validStart ? formatClockTime(start) : '--:--'}
        </span>
      }
      title={
        <span
          title={title}
          className={state === 'past-unrecorded' ? 'text-muted-foreground' : undefined}
        >
          {title}
        </span>
      }
      trailing={
        <>
          {/* Faces instead of nothing, as All Meetings has (owner feedback 2026-09-21). Week
              rows are the narrowest of the three views, so the cluster is capped tighter. The
              live meeting's reels badge is its StateChip. */}
          <AgendaAttendees item={item} max={2} className="hidden max-w-[200px] sm:flex" />
          <StateChip state={state} />
        </>
      }
      menu={<AgendaRowMenu item={item} ctx={ctx} actions={actions} />}
    />
  );
}

/**
 * Minimal week view (specs/0038 WS4): a vertical list of the seven Monday-started
 * days, each a card of compact meeting rows reusing the day view's phase styling
 * (`itemVisualState` / `StateChip`). Clicking a day header drops into its day view;
 * clicking a row routes exactly like the day timeline (`onSelectItem`).
 *
 * Since 2026-09-21 the rows also carry the attendee cluster, the recording marker and the
 * `…` menu the other two views have — the view was built read-only and the owner could see
 * a meeting here but not hide it. The day card is the List view's (`DayGroupCard`).
 */
export function WeekView({
  weekDays,
  weekItems,
  ctx,
  now,
  onSelectItem,
  onOpenDay,
  actions,
}: {
  weekDays: string[];
  weekItems: DayAgendaItem[][];
  ctx: TimelineContext;
  now: Date;
  onSelectItem: (item: DayAgendaItem) => void;
  onOpenDay: (dateKey: string) => void;
  actions: AgendaRowActions;
}) {
  const todayKey = localDateKey(now);
  return (
    <div className="flex flex-col gap-3 pb-2">
      {weekDays.map((dateKey, i) => {
        // Start-time order, whatever order the backend returned (audit 2026-11-11: a
        // Thursday listed 2:00 PM above 9:15 and 11:00).
        const dayItems = (weekItems[i] ?? []).filter((it) => !it.dismissed).sort(byStartTime);
        const d = parseLocalDateKey(dateKey);
        return (
          <DayGroupCard
            key={dateKey}
            label={d.toLocaleDateString([], { weekday: 'long' })}
            dateLabel={d.toLocaleDateString([], { month: 'short', day: 'numeric' })}
            count={dayItems.length}
            isToday={dateKey === todayKey}
            onOpen={() => onOpenDay(dateKey)}
          >
            {dayItems.length > 0
              ? dayItems.map((it) => (
                  <WeekRow
                    key={it.id}
                    item={it}
                    state={itemVisualState(it, ctx)}
                    ctx={ctx}
                    onSelect={onSelectItem}
                    actions={actions}
                  />
                ))
              : null}
          </DayGroupCard>
        );
      })}
    </div>
  );
}
