'use client';

import { useMemo } from 'react';
import type { DayAgendaItem } from '@/lib/day-agenda';
import {
  timelineBounds,
  timelineHeightPx,
  nowOffsetPx,
  hourOffsetPx,
  hourLabel,
  layoutTimeline,
  itemVisualState,
  canJoinItem,
  showsManualRecordButton,
  dayLabel,
  type TimelineBounds,
  type TimelineContext,
} from '@/lib/today-timeline';
import { TimelineBlock } from './TimelineBlock';

const GUTTER_PX = 56; // hour-label column width

interface DayTimelineProps {
  /** The day's agenda items with dismissed rows already filtered out. */
  visibleItems: DayAgendaItem[];
  now: Date;
  viewIsToday: boolean;
  viewDate: string;
  ctx: TimelineContext;
  onSelect: (item: DayAgendaItem) => void;
  onJoin: (item: DayAgendaItem) => void;
  onHide: (item: DayAgendaItem) => void;
  onEdit: (item: DayAgendaItem) => void;
  onDelete: (item: DayAgendaItem) => void;
  onRecord: (item: DayAgendaItem) => void;
}

/**
 * The single-day hour-grid timeline (specs/0036 WS7): hour gridlines + labels, the
 * "now" line (today only), and the laid-out item blocks. Pure presentation over the
 * `today-timeline` geometry helpers — data loading and click routing live with the
 * caller (Home).
 */
export function DayTimeline({
  visibleItems,
  now,
  viewIsToday,
  viewDate,
  ctx,
  onSelect,
  onJoin,
  onHide,
  onEdit,
  onDelete,
  onRecord,
}: DayTimelineProps) {
  // Only the today view carries a now-line, so only it stretches the grid to the current
  // hour; a navigated day fits just its own items (specs/0038 WS4).
  const bounds: TimelineBounds = useMemo(
    () => timelineBounds(visibleItems, now, viewIsToday),
    [visibleItems, now, viewIsToday],
  );
  // Lanes from true times + stack-don't-stagger push-down geometry (specs/0041 WS6).
  const laidOut = useMemo(() => layoutTimeline(visibleItems, bounds), [visibleItems, bounds]);

  const height = timelineHeightPx(bounds);
  const nowTop = nowOffsetPx(now, bounds);
  // The playhead reads its own time, the way a transport counter does.
  const nowLabel = now.toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' });
  const hours: number[] = [];
  for (let h = bounds.startHour; h <= bounds.endHour; h++) hours.push(h);

  return (
    <div className="relative flex" style={{ height }}>
      {/* Hour gridlines + labels */}
      <div className="relative flex-shrink-0" style={{ width: GUTTER_PX }}>
        {hours.map((h) => (
          /* Engraved hour stamp + a 1px tick running into the grid — the ruled edge of
             a transport log, not a floating caption (specs/0057 Task 6). */
          <div
            key={h}
            className="absolute right-0 flex -translate-y-1/2 items-center gap-1.5"
            style={{ top: hourOffsetPx(h, bounds) }}
          >
            <span className="font-narrow text-[10px] uppercase tracking-[0.06em] tabular-nums text-engrave">
              {hourLabel(h)}
            </span>
            <span aria-hidden="true" className="h-px w-2 bg-border" />
          </div>
        ))}
      </div>

      {/* Track: gridlines, now-line, and item blocks */}
      <div className="relative min-w-0 flex-1">
        {hours.map((h) => (
          <div
            key={h}
            className="absolute left-0 right-0 border-t border-border/50"
            style={{ top: hourOffsetPx(h, bounds) }}
            aria-hidden="true"
          />
        ))}

        {viewIsToday && nowTop >= 0 && nowTop <= height && (
          <div
            className="pointer-events-none absolute left-0 right-0 z-10 flex items-center"
            style={{ top: nowTop }}
            aria-hidden="true"
          >
            <span className="-ml-[3px] h-1.5 w-1.5 flex-shrink-0 bg-brand" />
            <span className="h-[1.5px] flex-1 bg-brand" />
            <span className="ml-1.5 font-narrow text-[10px] tabular-nums text-brand">
              {nowLabel}
            </span>
          </div>
        )}

        {laidOut.map(({ item, lane, laneCount, top, height }) => (
          <TimelineBlock
            key={item.id}
            item={item}
            state={itemVisualState(item, ctx)}
            top={top}
            height={height}
            lane={lane}
            laneCount={laneCount}
            canJoin={canJoinItem(item, ctx)}
            canRecord={showsManualRecordButton(item, ctx)}
            // The Hide/Edit/Delete gate moved into AgendaRowMenu (2026-09-21), which
            // derives it from the item + ctx — the week view was missing the menu
            // entirely because each view carried its own copy of these rules.
            ctx={ctx}
            onSelect={onSelect}
            onJoin={onJoin}
            onHide={onHide}
            onEdit={onEdit}
            onDelete={onDelete}
            onRecord={onRecord}
          />
        ))}

        {visibleItems.length === 0 && (
          <div className="absolute inset-x-0 top-[calc(50%-24px)] flex flex-col items-center justify-center text-center">
            <p className="text-sm font-medium text-foreground">
              {viewIsToday
                ? 'Nothing scheduled today'
                : `Nothing scheduled for ${dayLabel(viewDate, now)}`}
            </p>
            <p className="mt-1 max-w-xs text-xs text-muted-foreground">
              Hit Record to capture a meeting, or connect your calendar to see your day.
            </p>
          </div>
        )}
      </div>
    </div>
  );
}
