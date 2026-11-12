'use client';

/**
 * ActionItemRow (specs/0034) — one action item, shared by the per-meeting section
 * (`components/MeetingDetails/ActionItemsSection.tsx`) and the task hub
 * (`app/tasks/page.tsx`). The row is presentation-only: all IPC (and optimistic
 * state) lives in the parent via the callbacks.
 *
 * Affordances:
 *   - round checkbox: open ↔ completed;
 *   - description: click to edit inline (Enter/blur saves, Escape cancels) — or, when the
 *     surface passes `onOpen` (the To-do page), click opens the item's detail page;
 *     `@[Name](person:id)` mentions render as the person's current name;
 *   - assignee chip: AssigneePicker (roster/People + "Me" + free text + clear);
 *   - due hint: verbatim extracted text, display-only in v1; a structured due date that is
 *     today or overdue reads in `brand` (amber — never red, red means recording);
 *   - overflow menu: Dismiss (extracted — persists so re-extraction won't resurrect
 *     it), Restore (dismissed), Delete (manual — extracted items are dismissed, not
 *     deleted, or the next scan would just re-propose them).
 */

import { useState, type ReactNode } from 'react';
import { EyeOff, RotateCcw, Trash2 } from '@/components/ui/icons';
import { Checkbox } from '@/components/ui/checkbox';
import { MenuButton } from '@/components/ui/menu-button';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu';
import { cn } from '@/lib/utils';
import { titleSegments } from '@/lib/todo-parse';
import { Button } from '@/components/ui/button';
import type { ActionItem, ActionItemStatus } from '@/types';
import type { TriageAction } from '@/hooks/useActionItemMutations';
import { DueDateControl } from './DueDateControl';
import { MentionInput } from './MentionInput';
import { AssigneePicker, type AssigneeCandidate, type AssigneeSelection } from './AssigneePicker';

interface ActionItemRowProps {
  item: ActionItem;
  /** Resolved assignee display label ("Me" / name), or null when unassigned. */
  assigneeName: string | null;
  /** Pickable assignees for the chip (roster on the meeting page, People in the hub). */
  candidates: AssigneeCandidate[];
  onSetStatus: (item: ActionItem, status: ActionItemStatus) => void | Promise<void>;
  onEditDescription: (item: ActionItem, description: string) => void | Promise<void>;
  onAssign: (item: ActionItem, selection: AssigneeSelection) => void | Promise<void>;
  /** Hard delete — offered for manual items only. */
  onDelete: (item: ActionItem) => void | Promise<void>;
  /**
   * Set (ISO `YYYY-MM-DD`) or clear (`null`) the structured due date (WS1.a). When
   * omitted the row falls back to the read-only `dueHint` label (per-meeting section).
   */
  onSetDueDate?: (item: ActionItem, dueDate: string | null) => void | Promise<void>;
  /** Source-meeting deep-link chip (WS1.b flat views); rendered in the meta row. */
  meetingChip?: ReactNode;
  /** Drag handle (WS1.b Manual sort); rendered to the left of the checkbox. */
  dragHandle?: ReactNode;
  /** Open the item's detail page; when set, a title click calls this instead of editing. */
  onOpen?: (item: ActionItem) => void;
  /** Meetings that mention this item (specs/0088); "from N meetings" shows when above 1. */
  sourceCount?: number;
  /** Current person names by id, so mention tokens in the title show the live name. */
  nameById?: Map<string, string>;
  /**
   * Hide the assignee chip (specs/0063 W4). For a surface whose own grouping already
   * names the assignee — the Prep tab lists carried-over items under "Your open items"
   * and an "Owed by others → <name>" heading — the chip repeats that name on every row.
   * Reassignment there belongs on the meeting that owns the item, not in a prep view.
   */
  hideAssignee?: boolean;
  /**
   * Inline triage (specs/0088), used by the meeting page: a suggested row shows
   * Add to my list / Dismiss, a record row shows Adopt. Surfaces that omit it keep their
   * own triage controls.
   */
  onTriage?: (item: ActionItem, action: TriageAction) => void | Promise<void>;
  /** Name the item was adopted from; shown in the meta row as "adopted from {name}". */
  adoptedFrom?: string | null;
}

const EMPTY_NAMES = new Map<string, string>();

export function ActionItemRow({
  item,
  assigneeName,
  candidates,
  onSetStatus,
  onEditDescription,
  onAssign,
  onDelete,
  onSetDueDate,
  meetingChip,
  dragHandle,
  onOpen,
  sourceCount,
  nameById = EMPTY_NAMES,
  hideAssignee = false,
  onTriage,
  adoptedFrom = null,
}: ActionItemRowProps) {
  const [isEditing, setIsEditing] = useState(false);

  const completed = item.status === 'completed';
  const dismissed = item.status === 'dismissed';
  // Others' items are on the record only: nothing to tick off. With inline triage, a
  // suggestion isn't on the list yet either.
  const showCheckbox = item.triage !== 'record' && !(onTriage && item.triage === 'suggested');
  const suggestedInline = !!onTriage && item.triage === 'suggested' && !dismissed;
  const adoptInline = !!onTriage && item.triage === 'record' && !dismissed;
  // The menu holds exactly one entry per state; a suggested row's only one (Dismiss) is
  // inline, so there is nothing to open.
  const hasMenu = dismissed || item.source === 'manual' || !suggestedInline;

  const startEdit = () => setIsEditing(true);

  const commitEdit = (serialized: string) => {
    setIsEditing(false);
    const next = serialized.trim();
    if (next && next !== item.description) void onEditDescription(item, next);
  };

  const triageHandler = onTriage;
  const segments = titleSegments(item.description, nameById);

  return (
    <div className="group flex items-start gap-2 rounded-lg px-2 py-1.5 transition-colors hover:bg-accent/60">
      {dragHandle && <span className="-ml-6 shrink-0">{dragHandle}</span>}
      {showCheckbox ? (
        <Checkbox
          checked={completed}
          aria-label={
            completed
              ? `Mark "${item.description}" as open`
              : `Mark "${item.description}" as completed`
          }
          onCheckedChange={(next) => void onSetStatus(item, next ? 'completed' : 'open')}
          className="mt-0.5"
        />
      ) : (
        <span aria-hidden className="size-4 shrink-0" />
      )}

      <div className="min-w-0 flex-1">
        {isEditing ? (
          <MentionInput
            value={item.description}
            onSubmit={commitEdit}
            onCancel={() => setIsEditing(false)}
            submitOnBlur
            ariaLabel="Edit action item description"
            variant="bare"
            autoFocus
          />
        ) : (
          <div className="flex items-center gap-2">
            <button // ui-gate: allow click-to-edit description; it wraps across lines, which Button's nowrap fixed-height chrome can't
              type="button"
              onClick={() => (onOpen ? onOpen(item) : startEdit())}
              title={onOpen ? 'Open this item' : 'Edit description'}
              className={cn(
                'min-w-0 flex-1 rounded-md px-0.5 text-left text-sm leading-snug text-foreground hover:underline focus:outline-none focus-visible:ring-2 focus-visible:ring-ring',
                completed && 'text-muted-foreground line-through',
                dismissed && 'text-muted-foreground',
              )}
            >
              {segments.map((seg, i) =>
                seg.kind === 'person' ? (
                  <span key={i} className="font-medium">
                    @{seg.name}
                  </span>
                ) : (
                  <span key={i}>{seg.text}</span>
                ),
              )}
            </button>
          </div>
        )}

        <div className="mt-1 flex flex-wrap items-center gap-1.5">
          {!hideAssignee && (
            <AssigneePicker
              label={assigneeName}
              candidates={candidates}
              onSelect={(selection) => void onAssign(item, selection)}
            />
          )}
          {onSetDueDate ? (
            <DueDateControl item={item} onSetDueDate={onSetDueDate} />
          ) : (
            item.dueHint && (
              <span className="inline-flex items-center gap-1 text-caption text-muted-foreground">
                {item.dueHint}
              </span>
            )
          )}
          {sourceCount !== undefined && sourceCount > 1 && (
            <span className="text-caption text-muted-foreground">from {sourceCount} meetings</span>
          )}
          {adoptedFrom && (
            <span className="text-caption text-muted-foreground">adopted from {adoptedFrom}</span>
          )}
          {meetingChip}
          {triageHandler && suggestedInline && (
            <>
              <Button
                type="button"
                size="sm"
                variant="outline"
                onClick={() => void triageHandler(item, 'add_to_list')}
              >
                Add to my list
              </Button>
              <Button
                type="button"
                size="sm"
                variant="ghost"
                onClick={() => void triageHandler(item, 'dismiss')}
              >
                Dismiss
              </Button>
            </>
          )}
          {triageHandler && adoptInline && (
            <Button
              type="button"
              size="sm"
              variant="outline"
              onClick={() => void triageHandler(item, 'adopt')}
            >
              Adopt
            </Button>
          )}
          {dismissed && (
            <span className="u-section-label-sm rounded-sm bg-muted px-1.5 py-px">dismissed</span>
          )}
        </div>
      </div>

      {hasMenu && (
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <MenuButton
              label={`Options for "${item.description}"`}
              className="-my-0.5 opacity-0 transition-opacity focus-visible:opacity-100 group-hover:opacity-100 data-[state=open]:opacity-100"
            />
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end">
            {dismissed ? (
              <DropdownMenuItem icon={RotateCcw} onSelect={() => void onSetStatus(item, 'open')}>
                Restore
              </DropdownMenuItem>
            ) : item.source === 'manual' ? (
              <DropdownMenuItem icon={Trash2} variant="destructive" onSelect={() => void onDelete(item)}>
                Delete
              </DropdownMenuItem>
            ) : suggestedInline ? null : (
              <DropdownMenuItem icon={EyeOff} onSelect={() => void onSetStatus(item, 'dismissed')}>
                Dismiss
              </DropdownMenuItem>
            )}
          </DropdownMenuContent>
        </DropdownMenu>
      )}
    </div>
  );
}
