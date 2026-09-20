'use client';

import { useMemo } from 'react';
import { MoreHorizontal, EyeOff, Pencil, Trash2 } from 'lucide-react';
import { Button } from '@/components/ui/button';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu';
import type { DayAgendaItem } from '@/lib/day-agenda';
import { formatClockTime } from '@/lib/calendar';
import {
  itemVisualState,
  canJoinItem,
  canEditManualItem,
  canRecordManualItem,
  type TimelineContext,
} from '@/lib/today-timeline';
import { StateChip, barClass, blockClasses } from './TimelineBlock';

interface DayListProps {
  /** Already filtered of dismissed rows (the caller's `visibleItems`). */
  items: DayAgendaItem[];
  ctx: TimelineContext;
  now: Date;
  onSelect: (item: DayAgendaItem) => void;
  onJoin: (item: DayAgendaItem) => void;
  onRecord: (item: DayAgendaItem) => void;
  /** Unrecorded calendar item — offer "Hide from timeline" (mirrors `TimelineBlock`'s
   *  `canHide`/`onHide`; specs/0069 W4 fix round 1 — List is the DEFAULT view for
   *  no-calendar users, so it's the one place this affordance can't be missing). */
  onHide: (item: DayAgendaItem) => void;
  onEdit: (item: DayAgendaItem) => void;
  onDelete: (item: DayAgendaItem) => void;
  /** The empty state's own affordance out of a blank day. */
  onAddMeeting: () => void;
}

/**
 * The List presentation of Today's agenda (specs/0069 W4) — a plain, time-ordered
 * `<ul>` of the SAME `DayAgendaItem[]` the hour-grid timeline renders, for the machines
 * (no calendar connected) where an 8am-6pm grid mostly shows empty hours. It reuses the
 * grid's click-routing, `StateChip`, and per-state chrome/actions (`today-timeline.ts`,
 * `TimelineBlock`'s `barClass`/`blockClasses`) rather than reimplementing them — this is
 * a third presentation of the same rows, not a second behaviour.
 *
 * Field/action parity with `TimelineBlock` (fix round 1 — deliberate, not an oversight):
 *   - same: state chrome (`blockClasses`), state bar (`barClass`), `StateChip`, title
 *     fallback, time fallback, Join/Record buttons, the Edit/Delete/Hide ⋯ menu and its
 *     gate (`canHide || canEdit`), attendee count.
 *   - deliberately different: no lane/height geometry (a list row isn't positioned or
 *     variably sized, so there's no `compact` mode or absolute layout); the time renders
 *     in its own leading column instead of a meta line below the title, and the
 *     attendee count rides inline after the title instead of on that (nonexistent) meta
 *     line — both because a list row is single-line, not a multi-line block.
 */
export function DayList({
  items,
  ctx,
  onSelect,
  onJoin,
  onRecord,
  onHide,
  onEdit,
  onDelete,
  onAddMeeting,
}: DayListProps) {
  // The grid derives order from geometry (top offset); the list has no geometry, so it
  // sorts explicitly rather than trusting caller order.
  const sorted = useMemo(
    () => [...items].sort((a, b) => new Date(a.startTime).getTime() - new Date(b.startTime).getTime()),
    [items],
  );

  if (sorted.length === 0) {
    return (
      <div className="flex flex-col items-center gap-3 py-16 text-center">
        <p className="text-sm text-muted-foreground">Nothing today — add a meeting, or press record.</p>
        <Button variant="outline" onClick={onAddMeeting}>
          Add meeting
        </Button>
      </div>
    );
  }

  return (
    <ul className="flex flex-col gap-2">
      {sorted.map((item) => {
        const state = itemVisualState(item, ctx);
        const canJoin = canJoinItem(item, ctx);
        const canRecord = canRecordManualItem(item, ctx);
        const canEdit = canEditManualItem(item);
        // Same semantics as `DayTimeline`'s inline `canHide` (specs/0026): only an
        // UNrecorded calendar row — never a recording or the live session.
        const canHide =
          item.source === 'calendar' &&
          !item.status.recorded &&
          !item.meetingId &&
          ctx.recordingThisId !== item.id;
        const start = new Date(item.startTime);
        const validStart = !Number.isNaN(start.getTime());
        const title = item.title?.trim() || 'Untitled meeting';
        const bar = barClass(state);

        return (
          <li key={item.id}>
            <div
              role="button"
              tabIndex={0}
              onClick={() => onSelect(item)}
              onKeyDown={(e) => {
                if (e.key === 'Enter' || e.key === ' ') {
                  e.preventDefault();
                  onSelect(item);
                }
              }}
              title={title}
              className={`group relative flex cursor-pointer items-center gap-3 overflow-hidden rounded-[3px] border px-3 py-2.5 text-left transition-colors focus:outline-none focus-visible:ring-2 focus-visible:ring-ring ${blockClasses(
                state,
              )}`}
            >
              {bar && (
                <i aria-hidden="true" data-state-bar className={`absolute inset-y-0 left-0 w-[3px] ${bar}`} />
              )}
              <span className="u-meta w-16 flex-shrink-0 tabular-nums">
                {validStart ? formatClockTime(start) : '--:--'}
              </span>
              <span
                className={`min-w-0 flex-1 truncate text-[13.5px] font-semibold ${
                  state === 'past-unrecorded' ? 'text-muted-foreground' : 'text-foreground'
                }`}
              >
                {title}
                {item.attendeeCount > 0 && (
                  <span className="ml-1.5 truncate text-[12px] font-normal text-muted-foreground">
                    · {item.attendeeCount} attendee{item.attendeeCount === 1 ? '' : 's'}
                  </span>
                )}
              </span>
              {canJoin ? (
                <Button
                  variant="brand"
                  size="xs"
                  onClick={(e) => {
                    e.stopPropagation();
                    onJoin(item);
                  }}
                  className="flex-shrink-0"
                >
                  Join &amp; record
                </Button>
              ) : canRecord ? (
                <Button
                  variant="brand"
                  size="xs"
                  onClick={(e) => {
                    e.stopPropagation();
                    onRecord(item);
                  }}
                  className="flex-shrink-0"
                >
                  Record
                </Button>
              ) : (
                <StateChip state={state} />
              )}
              {(canHide || canEdit) && (
                <span
                  onClick={(e) => e.stopPropagation()}
                  onKeyDown={(e) => e.stopPropagation()}
                  className="flex-shrink-0"
                >
                  <DropdownMenu>
                    <DropdownMenuTrigger asChild>
                      <button
                        type="button"
                        aria-label="Event options"
                        title="Event options"
                        className="inline-flex h-5 w-5 items-center justify-center rounded-md text-muted-foreground opacity-0 transition-opacity hover:bg-accent hover:text-foreground focus:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:opacity-100 group-hover:opacity-100 data-[state=open]:opacity-100"
                      >
                        <MoreHorizontal className="h-3.5 w-3.5" />
                      </button>
                    </DropdownMenuTrigger>
                    <DropdownMenuContent align="end">
                      {canEdit && (
                        <>
                          <DropdownMenuItem onSelect={() => onEdit(item)}>
                            <Pencil className="mr-2 h-4 w-4" />
                            Edit
                          </DropdownMenuItem>
                          <DropdownMenuItem
                            onSelect={() => onDelete(item)}
                            className="text-destructive focus:text-destructive"
                          >
                            <Trash2 className="mr-2 h-4 w-4" />
                            Delete
                          </DropdownMenuItem>
                        </>
                      )}
                      {canHide && (
                        <DropdownMenuItem onSelect={() => onHide(item)}>
                          <EyeOff className="mr-2 h-4 w-4" />
                          Hide from timeline
                        </DropdownMenuItem>
                      )}
                    </DropdownMenuContent>
                  </DropdownMenu>
                </span>
              )}
            </div>
          </li>
        );
      })}
    </ul>
  );
}
