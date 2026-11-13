'use client';

/**
 * The To-do list surface (specs/0088), shared by the To-do page and a topic's action items
 * (specs/0089): the column header, the rows, and the keyboard model (`useTodoListKeys`) —
 * ↑↓ select, Space edit in place, ⌘D done, ⌘4 / ⌘5 highlight, Esc clear, and with
 * `reorder` also drag, ⌘↑↓ and ⌘1–3.
 *
 * The caller owns the rows and decides how long a ticked-off row stays: the To-do page
 * keeps it until you leave; a topic keeps it until you dismiss it there. Rows render in the
 * order given.
 */

import { useCallback, useEffect, useRef, useState, type ReactNode } from 'react';
import { DeckIcon } from '@/components/ui/deck-icon';
import { IconButton } from '@/components/ui/icon-button';
import { GripVertical } from '@/components/ui/icons';
import { REORDER_ATTR, useDragReorder } from '@/hooks/useDragReorder';
import { moveToIndex, useTodoListKeys, type Highlight } from '@/hooks/useTodoListKeys';
import { adoptedFromName } from '@/lib/action-items';
import { cn } from '@/lib/utils';
import type { ActionItem, ActionItemStatus, TodoRow } from '@/types';
import type { MentionTopic } from './MentionInput';
import { TodoListRow, todoGrid, todoRowDomId } from './TodoListRow';

export interface TodoListProps {
  rows: TodoRow[];
  ariaLabel: string;
  nameById: Map<string, string>;
  /** Open topics for the `@` picker. */
  topics?: MentionTopic[];
  /** Render the Topic column. Default: only when some row is tagged (specs/0089). */
  showTopic?: boolean;
  onSetStatus: (item: ActionItem, status: ActionItemStatus) => void | Promise<void>;
  onCommitEdit: (
    item: ActionItem,
    title: string,
    dueDate: string | null,
    topic?: MentionTopic | null,
  ) => void | Promise<void>;
  onSetDueDate: (item: ActionItem, dueDate: string | null) => void | Promise<void>;
  onDelete: (item: ActionItem) => void | Promise<void>;
  onOpen: (item: ActionItem) => void;
  onOpenTopic?: (topicId: string) => void;
  onToggleHighlight: (id: string, color: Highlight) => void;
  /** Persist a new order (ids in list order). Omit for a list that follows another's order. */
  reorder?: (ids: string[]) => void;
  /** A ticked-off row's dismiss action (a topic's "Dismiss from this topic"). */
  onDismissDone?: (item: ActionItem) => void;
  /** Extra classes on the listbox. */
  className?: string;
}

/** The keyboard legend under a list, matching what it supports. */
export function todoKeysHint(canReorder: boolean): string {
  return canReorder
    ? '↑↓ select · Space edit · ⌘D done · ⌘↑↓ move · ⌘1–3 move to 1st–3rd · ⌘4 / ⌘5 highlight'
    : '↑↓ select · Space edit · ⌘D done · ⌘4 / ⌘5 highlight';
}

const NO_REORDER = () => {};

export function TodoList({
  rows,
  ariaLabel,
  nameById,
  topics,
  showTopic: showTopicProp,
  onSetStatus,
  onCommitEdit,
  onSetDueDate,
  onDelete,
  onOpen,
  onOpenTopic,
  onToggleHighlight,
  reorder,
  onDismissDone,
  className,
}: TodoListProps) {
  const ids = rows.map((r) => r.id);
  const { draggingId, overId, gripProps } = useDragReorder(ids, reorder ?? NO_REORDER);

  const listRef = useRef<HTMLDivElement>(null);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [editingId, setEditingId] = useState<string | null>(null);
  const selectRow = useCallback((id: string | null) => {
    setSelectedId(id);
    if (id) document.getElementById(todoRowDomId(id))?.scrollIntoView({ block: 'nearest' });
  }, []);
  // Hand focus back to the list after an edit so the keys keep working.
  const endEdit = useCallback(() => {
    setEditingId(null);
    requestAnimationFrame(() => listRef.current?.focus({ preventScroll: true }));
  }, []);
  // A click anywhere outside the list releases the selection — except inside a row's own
  // popovers (date picker, ⋯ menu), which render in a portal outside the list's DOM.
  useEffect(() => {
    if (!selectedId) return;
    const onPointerDown = (e: PointerEvent) => {
      const t = e.target as Element | null;
      if (!t || listRef.current?.contains(t)) return;
      if (t.closest('[data-radix-popper-content-wrapper], [role="menu"], [role="dialog"]')) return;
      setSelectedId(null);
      setEditingId(null);
    };
    document.addEventListener('pointerdown', onPointerDown);
    return () => document.removeEventListener('pointerdown', onPointerDown);
  }, [selectedId]);
  // A row that leaves the list (dismissed, deleted) takes the selection with it.
  const idsKey = ids.join('\n');
  useEffect(() => {
    if (selectedId && !idsKey.split('\n').includes(selectedId)) setSelectedId(null);
  }, [idsKey, selectedId]);

  // Auto Topic column: there only when some row is tagged. While an inline edit is open the
  // layout holds still, so a re-tag landing mid-edit can't shift the field.
  const anyTagged = rows.some((i) => !!i.topicId);
  const heldTopicColumn = useRef(anyTagged);
  if (editingId === null || anyTagged) heldTopicColumn.current = anyTagged;
  const showTopic = showTopicProp ?? heldTopicColumn.current;

  const toggleDone = useCallback(
    (id: string) => {
      const row = rows.find((r) => r.id === id);
      if (row) void onSetStatus(row, row.status === 'completed' ? 'open' : 'completed');
    },
    [rows, onSetStatus],
  );

  const onKeyDown = useTodoListKeys({
    ids,
    selectedId,
    editing: editingId !== null,
    select: selectRow,
    startEdit: setEditingId,
    reorder: reorder ?? NO_REORDER,
    toggleHighlight: onToggleHighlight,
    toggleDone,
  });

  const grip = (id: string): ReactNode =>
    reorder ? (
      <IconButton
        variant="ghost"
        label="Drag to reorder"
        {...gripProps(id)}
        icon={<DeckIcon icon={GripVertical} size={14} />}
        // A narrow grip in the row's gutter, not a 28px key: it shows on row hover and
        // reads as a handle, not a button.
        className={cn(
          'h-[18px] w-4 cursor-grab touch-none text-muted-foreground/40 opacity-0 transition-opacity hover:bg-transparent hover:text-muted-foreground focus-visible:opacity-100 group-hover:opacity-100 active:cursor-grabbing',
          draggingId === id && 'opacity-100',
        )}
      />
    ) : undefined;

  return (
    <div
      ref={listRef}
      className={cn('group/list px-1 focus:outline-none', className)}
      role="listbox"
      aria-label={ariaLabel}
      tabIndex={0}
      aria-activedescendant={selectedId ? todoRowDomId(selectedId) : undefined}
      onKeyDown={onKeyDown}
    >
      <div className={cn(todoGrid(showTopic), 'px-2 pb-1')} aria-hidden>
        <span />
        <span className="u-section-label-sm">Item</span>
        {showTopic ? <span className="u-section-label-sm">Topic</span> : null}
        <span className="u-section-label-sm">Due</span>
      </div>
      {rows.map((item) => (
        <div
          key={item.id}
          role="presentation"
          {...(reorder ? { [REORDER_ATTR]: item.id } : {})}
          className={cn(
            'rounded-lg transition-shadow',
            draggingId === item.id && 'opacity-50',
            overId === item.id && draggingId && draggingId !== item.id && 'ring-2 ring-brand-edge',
          )}
        >
          <TodoListRow
            item={item}
            nameById={nameById}
            sourceCount={item.sourceCount}
            adoptedFrom={adoptedFromName(item, nameById)}
            highlight={item.highlight ?? null}
            selected={selectedId === item.id}
            editing={editingId === item.id}
            onSelect={(row) => {
              selectRow(row.id);
              listRef.current?.focus({ preventScroll: true });
            }}
            onStartEdit={(row) => setEditingId(row.id)}
            onCommitEdit={(row, title, due, topic) => {
              endEdit();
              void onCommitEdit(row, title, due, topic);
            }}
            topics={topics}
            showTopic={showTopic}
            onOpenTopic={onOpenTopic}
            onCancelEdit={endEdit}
            onToggleHighlight={(row, color) => onToggleHighlight(row.id, color)}
            onMoveToTop={reorder ? (row) => reorder(moveToIndex(ids, row.id, 0)) : undefined}
            onOpen={onOpen}
            onSetStatus={onSetStatus}
            onSetDueDate={onSetDueDate}
            onDelete={onDelete}
            onDismissDone={onDismissDone}
            dragHandle={grip(item.id)}
          />
        </div>
      ))}
    </div>
  );
}
