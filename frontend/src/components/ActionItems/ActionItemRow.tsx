'use client';

/**
 * ActionItemRow (specs/0034) — one action item, shared by the per-meeting section
 * (`components/MeetingDetails/ActionItemsSection.tsx`) and the task hub
 * (`app/tasks/page.tsx`). The row is presentation-only: all IPC (and optimistic
 * state) lives in the parent via the callbacks.
 *
 * Affordances:
 *   - round checkbox: open ↔ completed;
 *   - description: click to edit inline (Enter/blur saves, Escape cancels);
 *   - assignee chip: AssigneePicker (roster/People + "Me" + free text + clear);
 *   - due hint: verbatim extracted text, display-only in v1;
 *   - overflow menu: Dismiss (extracted — persists so re-extraction won't resurrect
 *     it), Restore (dismissed), Delete (manual — extracted items are dismissed, not
 *     deleted, or the next scan would just re-propose them).
 */

import { useState, type ReactNode } from 'react';
import { CalendarPlus, Check, Clock, EyeOff, MoreHorizontal, RotateCcw, Trash2 } from 'lucide-react';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu';
import { cn } from '@/lib/utils';
import { formatDueDate } from '@/lib/format-date';
import type { ActionItem, ActionItemStatus } from '@/types';
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
  /**
   * Collapse to a single line by default, expanding the meta row on hover/focus
   * (WS1.b hub flat views). The per-meeting section leaves this false.
   */
  compact?: boolean;
}

/**
 * Compact due-date control (WS1.a): shows the structured `dueDate` (or `dueHint` as a
 * fallback label when there's no date), and reveals a native date input to set/clear it.
 * A native `<input type="date">` keeps this dependency-free and works in the WKWebview.
 */
function DueDateControl({
  item,
  onSetDueDate,
}: {
  item: ActionItem;
  onSetDueDate: (item: ActionItem, dueDate: string | null) => void | Promise<void>;
}) {
  const label = formatDueDate(item.dueDate) ?? item.dueHint;
  return (
    <label
      className={cn(
        'relative inline-flex cursor-pointer items-center gap-1 rounded px-1 py-px text-[11px] transition-colors hover:bg-muted focus-within:ring-2 focus-within:ring-ring',
        label ? 'text-muted-foreground' : 'text-muted-foreground/60',
      )}
      title={item.dueDate ? 'Change due date' : 'Set a due date'}
    >
      {label ? <Clock size={11} /> : <CalendarPlus size={11} />}
      <span>{label ?? 'Due'}</span>
      {/* The native picker overlays the label; onChange sets, empty value clears. */}
      <input
        type="date"
        aria-label="Due date"
        value={item.dueDate ?? ''}
        onChange={(e) => void onSetDueDate(item, e.target.value ? e.target.value : null)}
        className="absolute inset-0 cursor-pointer opacity-0"
      />
    </label>
  );
}

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
  compact = false,
}: ActionItemRowProps) {
  const [isEditing, setIsEditing] = useState(false);
  const [draft, setDraft] = useState(item.description);

  const completed = item.status === 'completed';
  const dismissed = item.status === 'dismissed';

  const startEdit = () => {
    setDraft(item.description);
    setIsEditing(true);
  };

  const commitEdit = () => {
    setIsEditing(false);
    const next = draft.trim();
    if (next && next !== item.description) void onEditDescription(item, next);
  };

  const dueLabel = formatDueDate(item.dueDate) ?? item.dueHint;
  // Collapsed one-line summary (compact mode, not hovered): a muted glance at assignee
  // and due so the row still reads at a glance before it expands.
  const collapsedMeta =
    compact && (assigneeName || dueLabel)
      ? [assigneeName, dueLabel].filter(Boolean).join(' · ')
      : null;

  return (
    <div className="group flex items-start gap-2 rounded-lg px-2 py-1.5 transition-colors hover:bg-accent/60">
      {dragHandle}
      <button
        type="button"
        role="checkbox"
        aria-checked={completed}
        aria-label={
          completed
            ? `Mark "${item.description}" as open`
            : `Mark "${item.description}" as completed`
        }
        onClick={() => void onSetStatus(item, completed ? 'open' : 'completed')}
        className={cn(
          'mt-0.5 flex h-[18px] w-[18px] flex-shrink-0 items-center justify-center rounded-[2px] border transition-colors focus:outline-none focus-visible:ring-2 focus-visible:ring-ring',
          completed
            ? 'border-brand bg-brand text-brand-foreground'
            : 'border-border bg-card hover:border-brand/60',
        )}
      >
        {completed && <Check size={12} strokeWidth={3} />}
      </button>

      <div className="min-w-0 flex-1">
        {isEditing ? (
          <input
            value={draft}
            onChange={(e) => setDraft(e.target.value)}
            onBlur={commitEdit}
            onKeyDown={(e) => {
              if (e.key === 'Enter') {
                e.preventDefault();
                commitEdit();
              } else if (e.key === 'Escape') {
                setDraft(item.description);
                setIsEditing(false);
              }
            }}
            aria-label="Edit action item description"
            className="w-full rounded border border-border bg-background px-1.5 py-0.5 text-sm text-foreground focus:outline-none focus-visible:ring-2 focus-visible:ring-ring"
            autoFocus
          />
        ) : (
          <div className="flex items-center gap-2">
            <button
              type="button"
              onClick={startEdit}
              title="Edit description"
              className={cn(
                'min-w-0 flex-1 rounded px-0.5 text-left text-sm leading-snug text-foreground hover:text-brand focus:outline-none focus-visible:ring-2 focus-visible:ring-ring',
                compact && 'truncate',
                completed && 'text-muted-foreground line-through',
                dismissed && 'text-muted-foreground',
              )}
            >
              {item.description}
            </button>
            {collapsedMeta && (
              <span className="flex-shrink-0 truncate text-[11px] text-muted-foreground group-hover:hidden group-focus-within:hidden">
                {collapsedMeta}
              </span>
            )}
          </div>
        )}

        <div
          className={cn(
            'mt-1 flex-wrap items-center gap-1.5',
            compact
              ? 'hidden group-hover:flex group-focus-within:flex'
              : 'flex',
          )}
        >
          <AssigneePicker
            label={assigneeName}
            candidates={candidates}
            onSelect={(selection) => void onAssign(item, selection)}
          />
          {onSetDueDate ? (
            <DueDateControl item={item} onSetDueDate={onSetDueDate} />
          ) : (
            item.dueHint && (
              <span className="inline-flex items-center gap-1 text-[11px] text-muted-foreground">
                <Clock size={11} />
                {item.dueHint}
              </span>
            )
          )}
          {meetingChip}
          {dismissed && (
            <span className="rounded-[2px] bg-muted px-1.5 py-px u-section-label text-[9px]">
              dismissed
            </span>
          )}
        </div>
      </div>

      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <button
            type="button"
            aria-label={`Options for "${item.description}"`}
            className="mt-0.5 inline-flex h-6 w-6 flex-shrink-0 items-center justify-center rounded-md text-muted-foreground opacity-0 transition-opacity hover:bg-muted hover:text-foreground focus:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:opacity-100 group-hover:opacity-100 data-[state=open]:opacity-100"
          >
            <MoreHorizontal className="h-4 w-4" />
          </button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end">
          {dismissed ? (
            <DropdownMenuItem onSelect={() => void onSetStatus(item, 'open')}>
              <RotateCcw className="mr-2 h-4 w-4" />
              Restore
            </DropdownMenuItem>
          ) : item.source === 'manual' ? (
            <DropdownMenuItem
              onSelect={() => void onDelete(item)}
              className="text-destructive focus:text-destructive"
            >
              <Trash2 className="mr-2 h-4 w-4" />
              Delete
            </DropdownMenuItem>
          ) : (
            <DropdownMenuItem onSelect={() => void onSetStatus(item, 'dismissed')}>
              <EyeOff className="mr-2 h-4 w-4" />
              Dismiss
            </DropdownMenuItem>
          )}
        </DropdownMenuContent>
      </DropdownMenu>
    </div>
  );
}
