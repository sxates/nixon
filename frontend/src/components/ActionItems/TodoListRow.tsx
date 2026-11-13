'use client';

/**
 * One row of the To-do tab (specs/0088), laid out in columns so each item takes a
 * single line where its title fits: grip · checkbox · title · Due · open/menu. The column
 * template lives in `TODO_GRID` so the list header lines up with the rows; a Topics
 * column (specs/0089) slots in before Due.
 *
 * Selection and editing are driven by the list (see `useTodoListKeys`): a click on the
 * row selects it, a click on the title of the selected row edits it in place (Enter
 * saves, Esc cancels, a trailing "(Friday)" becomes the due date). The chevron opens the
 * detail page. A highlight (⌘4 yellow / ⌘5 green) marks the title like a highlighter pen.
 */

import type { ReactNode } from 'react';
import { ChevronRight, EyeOff, Trash2 } from '@/components/ui/icons';
import { Checkbox } from '@/components/ui/checkbox';
import { DeckIcon } from '@/components/ui/deck-icon';
import { IconButton } from '@/components/ui/icon-button';
import { MenuButton } from '@/components/ui/menu-button';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuShortcut,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu';
import type { Highlight } from '@/hooks/useTodoListKeys';
import { parseTodoText, titleSegments } from '@/lib/todo-parse';
import { cn } from '@/lib/utils';
import type { ActionItem, ActionItemStatus } from '@/types';
import { DueDateControl } from './DueDateControl';
import { MentionInput } from './MentionInput';

/**
 * checkbox · title · due · actions — shared by the header and every row. Every cell's
 * content starts at x=0 of its track (no inset padding), so the header labels sit exactly
 * over the text below, and every cell centres in a 20px band matching the title's first
 * line (`text-sm leading-5`), so a long title wraps downward without dragging the
 * checkbox or the date off its first line.
 */
export const TODO_GRID =
  'grid grid-cols-[1rem_minmax(0,1fr)_7.5rem_3.5rem] items-start gap-x-3';

/** Highlighter-pen washes on the title text. */
const HIGHLIGHT_CLASS: Record<Highlight, string> = {
  yellow: 'bg-highlight-yellow',
  green: 'bg-highlight-green',
};

export const todoRowDomId = (id: string) => `todo-row-${id}`;

interface TodoListRowProps {
  item: ActionItem;
  nameById: Map<string, string>;
  sourceCount?: number;
  adoptedFrom?: string | null;
  highlight?: Highlight | null;
  selected?: boolean;
  editing?: boolean;
  dragHandle?: ReactNode;
  onSelect: (item: ActionItem) => void;
  onStartEdit: (item: ActionItem) => void;
  /** Enter / blur in the edit field: the title with any trailing "(date)" split off. */
  onCommitEdit: (item: ActionItem, title: string, dueDate: string | null) => void;
  onCancelEdit: () => void;
  onOpen: (item: ActionItem) => void;
  onSetStatus: (item: ActionItem, status: ActionItemStatus) => void | Promise<void>;
  onSetDueDate: (item: ActionItem, dueDate: string | null) => void | Promise<void>;
  onDelete: (item: ActionItem) => void | Promise<void>;
  onToggleHighlight: (item: ActionItem, color: Highlight) => void;
  onMoveToTop: (item: ActionItem) => void;
}

export function TodoListRow({
  item,
  nameById,
  sourceCount,
  adoptedFrom = null,
  highlight = null,
  selected = false,
  editing = false,
  dragHandle,
  onSelect,
  onStartEdit,
  onCommitEdit,
  onCancelEdit,
  onOpen,
  onSetStatus,
  onSetDueDate,
  onDelete,
  onToggleHighlight,
  onMoveToTop,
}: TodoListRowProps) {
  const completed = item.status === 'completed';
  const meta = [
    sourceCount !== undefined && sourceCount > 1 ? `from ${sourceCount} meetings` : null,
    adoptedFrom ? `adopted from ${adoptedFrom}` : null,
  ].filter(Boolean);

  const commit = (serialized: string) => {
    const { title, dueDate } = parseTodoText(serialized.trim(), new Date());
    onCommitEdit(item, title, dueDate);
  };

  return (
    <div
      id={todoRowDomId(item.id)}
      role="option"
      aria-selected={selected}
      onClick={() => onSelect(item)}
      className={cn(
        TODO_GRID,
        'group relative rounded-lg px-2 py-1 transition-colors',
        selected
          ? 'bg-accent ring-1 ring-inset ring-rule group-focus/list:ring-brand-edge'
          : 'hover:bg-accent/60',
      )}
    >
      {dragHandle && <span className="absolute -left-4 top-1 flex h-5 items-center">{dragHandle}</span>}
      <span className="flex h-5 items-center">
        <Checkbox
          checked={completed}
          aria-label={
            completed ? `Mark "${item.description}" as open` : `Mark "${item.description}" as completed`
          }
          onCheckedChange={(next) => void onSetStatus(item, next ? 'completed' : 'open')}
        />
      </span>

      <div className="min-w-0 text-sm leading-5">
        {editing ? (
          <MentionInput
            value={item.description}
            onSubmit={commit}
            onCancel={onCancelEdit}
            submitOnBlur
            multiline
            autoFocus
            variant="bare"
            ariaLabel="Edit to-do"
            className="-mx-1 -my-0.5 leading-5"
          />
        ) : (
          <>
            <button // ui-gate: allow title text; it wraps across lines, which Button's nowrap fixed-height chrome can't
              type="button"
              onClick={(e) => {
                e.stopPropagation();
                if (selected) onStartEdit(item);
                else onSelect(item);
              }}
              title={selected ? 'Click to edit' : undefined}
              className={cn(
                'cursor-text rounded-sm text-left text-foreground focus:outline-none focus-visible:ring-2 focus-visible:ring-ring',
                completed && 'text-muted-foreground line-through',
              )}
            >
              <span
                className={cn(
                  highlight && ['-mx-1 rounded-sm px-1 py-0.5 box-decoration-clone', HIGHLIGHT_CLASS[highlight]],
                )}
              >
                {titleSegments(item.description, nameById).map((seg, i) =>
                  seg.kind === 'person' ? (
                    <span key={i} className="font-medium">
                      @{seg.name}
                    </span>
                  ) : (
                    <span key={i}>{seg.text}</span>
                  ),
                )}
              </span>
            </button>
            {meta.length > 0 && (
              <span className="ml-2 text-caption text-muted-foreground">{meta.join(' · ')}</span>
            )}
          </>
        )}
      </div>

      <span className="flex h-5 min-w-0 items-center">
        {/* -ml-1 cancels the control's hover padding so the date sits under the header. */}
        <DueDateControl item={item} onSetDueDate={onSetDueDate} quietWhenEmpty className="-ml-1 max-w-full" />
      </span>

      <span className="flex h-5 items-center justify-end gap-0.5">
        <IconButton
          variant="ghost"
          size="sm"
          label="Open details"
          icon={<DeckIcon icon={ChevronRight} size={14} />}
          onClick={(e) => {
            e.stopPropagation();
            onOpen(item);
          }}
          className={cn(
            'opacity-0 transition-opacity focus-visible:opacity-100 group-hover:opacity-100',
            selected && 'opacity-100',
          )}
        />
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <MenuButton
              label={`Options for "${item.description}"`}
              className="opacity-0 transition-opacity focus-visible:opacity-100 group-hover:opacity-100 data-[state=open]:opacity-100"
            />
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end" onClick={(e) => e.stopPropagation()}>
            <DropdownMenuItem onSelect={() => onMoveToTop(item)}>
              Move to top <DropdownMenuShortcut>⌘1</DropdownMenuShortcut>
            </DropdownMenuItem>
            <DropdownMenuItem onSelect={() => onToggleHighlight(item, 'yellow')}>
              {highlight === 'yellow' ? 'Remove highlight' : 'Highlight yellow'}
              <DropdownMenuShortcut>⌘4</DropdownMenuShortcut>
            </DropdownMenuItem>
            <DropdownMenuItem onSelect={() => onToggleHighlight(item, 'green')}>
              {highlight === 'green' ? 'Remove highlight' : 'Highlight green'}
              <DropdownMenuShortcut>⌘5</DropdownMenuShortcut>
            </DropdownMenuItem>
            <DropdownMenuSeparator />
            {item.source === 'manual' ? (
              <DropdownMenuItem icon={Trash2} variant="destructive" onSelect={() => void onDelete(item)}>
                Delete
              </DropdownMenuItem>
            ) : (
              <DropdownMenuItem icon={EyeOff} onSelect={() => void onSetStatus(item, 'dismissed')}>
                Dismiss
              </DropdownMenuItem>
            )}
          </DropdownMenuContent>
        </DropdownMenu>
      </span>
    </div>
  );
}
