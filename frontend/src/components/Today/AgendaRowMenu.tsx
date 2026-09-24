'use client';

/**
 * The `…` menu on a Today row — Record now / Edit / Delete for a manually added meeting,
 * Hide for an unrecorded calendar event.
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

import { Circle, EyeOff, MoreHorizontal, Pencil, Trash2 } from 'lucide-react';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu';
import type { DayAgendaItem } from '@/lib/day-agenda';
import {
  canDeleteRecordedItem,
  canEditManualItem,
  canRecordManualItem,
  showsManualRecordButton,
  type TimelineContext,
} from '@/lib/today-timeline';

export interface AgendaRowActions {
  onHide: (item: DayAgendaItem) => void;
  onEdit: (item: DayAgendaItem) => void;
  onDelete: (item: DayAgendaItem) => void;
  onRecord: (item: DayAgendaItem) => void;
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
  const canDelete = canDeleteRecordedItem(item, ctx);
  // Early recording of a manual meeting: before its start window the row shows Prep, not
  // the Record button, so this is where recording it ahead of time lives (2026-09-24).
  const canRecordEarly = canRecordManualItem(item, ctx) && !showsManualRecordButton(item, ctx);
  // Nothing to offer — but keep the slot, so the chips of rows with and without a menu line
  // up (owner report 2026-09-23: the Week view's RECORDED chips stepped in and out).
  if (!canEdit && !canHide && !canDelete && !canRecordEarly) return <span aria-hidden="true" className="h-5 w-5 flex-shrink-0" />;

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
          {canRecordEarly && (
            <DropdownMenuItem onSelect={() => actions.onRecord(item)}>
              <Circle className="mr-2 h-4 w-4" />
              Record now
            </DropdownMenuItem>
          )}
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
          {canDelete && (
            <DropdownMenuItem
              onSelect={() => actions.onDelete(item)}
              className="text-destructive focus:text-destructive"
            >
              <Trash2 className="mr-2 h-4 w-4" />
              Delete meeting
            </DropdownMenuItem>
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
