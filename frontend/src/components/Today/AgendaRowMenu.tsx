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

import { Circle, EyeOff, Pencil, Trash2 } from '@/components/ui/icons';
import { MenuButton } from '@/components/ui/menu-button';
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
import { cn } from '@/lib/utils';

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
  triggerClassName,
}: {
  item: DayAgendaItem;
  ctx: TimelineContext;
  actions: AgendaRowActions;
  /**
   * Extra classes for the trigger (and the empty slot that stands in for it). A `ListRow`
   * reveals its menu on hover by itself; the day grid's blocks pass their own reveal and a
   * negative margin so the 28px key doesn't grow a 20px title line.
   */
  triggerClassName?: string;
}) {
  const canEdit = canEditManualItem(item);
  const canHide = canHideItem(item, ctx);
  const canDelete = canDeleteRecordedItem(item, ctx);
  // Early recording of a manual meeting: before its start window the row shows Prep, not
  // the Record button, so this is where recording it ahead of time lives (2026-09-24).
  const canRecordEarly = canRecordManualItem(item, ctx) && !showsManualRecordButton(item, ctx);
  // Nothing to offer — but keep the slot, so the chips of rows with and without a menu line
  // up (owner report 2026-09-23: the Week view's RECORDED chips stepped in and out).
  if (!canEdit && !canHide && !canDelete && !canRecordEarly) {
    return <span aria-hidden="true" className={cn('h-7 w-7 flex-none', triggerClassName)} />;
  }

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
          <MenuButton label="Event options" className={triggerClassName} />
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end">
          {canRecordEarly && (
            <DropdownMenuItem icon={Circle} onSelect={() => actions.onRecord(item)}>
              Record now
            </DropdownMenuItem>
          )}
          {canEdit && (
            <>
              <DropdownMenuItem icon={Pencil} onSelect={() => actions.onEdit(item)}>
                Edit
              </DropdownMenuItem>
              <DropdownMenuItem icon={Trash2} variant="destructive" onSelect={() => actions.onDelete(item)}>
                Delete
              </DropdownMenuItem>
            </>
          )}
          {canDelete && (
            <DropdownMenuItem icon={Trash2} variant="destructive" onSelect={() => actions.onDelete(item)}>
              Delete meeting
            </DropdownMenuItem>
          )}
          {canHide && (
            <DropdownMenuItem icon={EyeOff} onSelect={() => actions.onHide(item)}>
              Hide from timeline
            </DropdownMenuItem>
          )}
        </DropdownMenuContent>
      </DropdownMenu>
    </span>
  );
}
