'use client';

/**
 * The `…` menu on a Today row — Edit / Delete for a manually added meeting, Hide for an
 * unrecorded calendar event.
 *
 * Owner feedback 2026-09-21: "On Today, the 'week' view is missing the … actions menu that
 * lets me remove items." It was missing because `WeekView` was written (specs/0038 WS4) as
 * a read-only overview and never grew the affordance the day grid and the list both have.
 * Rather than write a third copy, the menu — and the `canEdit`/`canHide` rules that decide
 * what it offers — live here, so the three views cannot drift again.
 *
 * Renders nothing when neither action applies, so a recorded meeting's row carries no
 * dead control.
 */

import { EyeOff, MoreHorizontal, Pencil, Trash2 } from 'lucide-react';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu';
import type { DayAgendaItem } from '@/lib/day-agenda';
import {
  canEditManualItem,
  type TimelineContext,
} from '@/lib/today-timeline';

export interface AgendaRowActions {
  onHide: (item: DayAgendaItem) => void;
  onEdit: (item: DayAgendaItem) => void;
  onDelete: (item: DayAgendaItem) => void;
}

/**
 * Can this row be hidden from the agenda? Only an UNRECORDED calendar event: hiding a
 * recording would be deleting it, and a manually added meeting has Delete instead
 * (specs/0026, and `calendar/manual_items.rs` asserts the same rule on the Rust side).
 */
export function canHideItem(item: DayAgendaItem, ctx: TimelineContext): boolean {
  return (
    item.source === 'calendar' &&
    !item.status.recorded &&
    !item.meetingId &&
    ctx.recordingThisId !== item.id
  );
}

export function AgendaRowMenu({
  item,
  ctx,
  actions,
}: {
  item: DayAgendaItem;
  ctx: TimelineContext;
  actions: AgendaRowActions;
}) {
  const canEdit = canEditManualItem(item);
  const canHide = canHideItem(item, ctx);
  if (!canEdit && !canHide) return null;

  return (
    // The row itself is clickable (it opens the meeting), so the menu swallows its own
    // clicks and keys — otherwise opening the menu would also navigate away from it.
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
              <DropdownMenuItem onSelect={() => actions.onEdit(item)}>
                <Pencil className="mr-2 h-4 w-4" />
                Edit
              </DropdownMenuItem>
              <DropdownMenuItem
                onSelect={() => actions.onDelete(item)}
                className="text-destructive focus:text-destructive"
              >
                <Trash2 className="mr-2 h-4 w-4" />
                Delete
              </DropdownMenuItem>
            </>
          )}
          {canHide && (
            <DropdownMenuItem onSelect={() => actions.onHide(item)}>
              <EyeOff className="mr-2 h-4 w-4" />
              Hide from timeline
            </DropdownMenuItem>
          )}
        </DropdownMenuContent>
      </DropdownMenu>
    </span>
  );
}
