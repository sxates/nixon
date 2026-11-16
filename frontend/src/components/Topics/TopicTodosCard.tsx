'use client';

/**
 * My action items (specs/0089 §Action items integration): the to-dos tagged to this topic,
 * as the same list the To-do page uses — quick-add on top, then ↑↓ select, Space edit, ⌘D
 * done, ⌘4 / ⌘5 highlight, and drag / ⌘↑↓ / ⌘1–3 to reorder (specs/0090). A reorder here
 * swaps the topic's items among the places they already hold on your To-do list, so items
 * from other topics never move. A ticked-off item stays on the card, struck through, until
 * you dismiss it (its ✕, or "Dismiss done" for all of them); it is done everywhere the
 * moment you tick it. The header's "Show in To-Do" switch (specs/0090, on by default) is
 * the topic's `hideFromTodo` flipped: off keeps its open to-dos here and off the To-do list.
 */

import { useCallback, useEffect, useMemo, useState } from 'react';
import { useRouter } from 'next/navigation';
import { toast } from 'sonner';
import { AddActionItemRow } from '@/components/ActionItems/AddActionItemRow';
import { TodoList, todoKeysHint } from '@/components/ActionItems/TodoList';
import type { MentionTopic } from '@/components/ActionItems/MentionInput';
import { Button } from '@/components/ui/button';
import { Label } from '@/components/ui/label';
import { Switch } from '@/components/ui/switch';
import { Panel } from '@/components/ui/panel';
import { useActionItemMutations } from '@/hooks/useActionItemMutations';
import { usePersonNames } from '@/hooks/usePersonNames';
import { useTodoHighlight } from '@/hooks/useTodoHighlight';
import { useTopics } from '@/hooks/useTopics';
import { sameOrder } from '@/lib/topic-order';
import { clearTopicTodos, setActionItemTopic, updateTopic } from '@/lib/topics';
import type { ActionItem, ActionItemStatus, TodoRow } from '@/types';

const errorText = (err: unknown) => (err instanceof Error ? err.message : String(err));

interface TopicTodosCardProps {
  topicId: string;
  todos: TodoRow[];
  /** Refetch the topic after a change. */
  onChanged: () => void | Promise<void>;
  /** The topic's `hideFromTodo`; the "Show in To-Do" switch is its inverse (specs/0090). */
  hideFromTodo?: boolean;
}

const bySortOrder = (a: TodoRow, b: TodoRow) => (a.sortOrder ?? Infinity) - (b.sortOrder ?? Infinity);

export function TopicTodosCard({ topicId, todos, onChanged, hideFromTodo = false }: TopicTodosCardProps) {
  const router = useRouter();
  // Mirrored locally so ticking, editing and highlighting are optimistic.
  const [items, setItems] = useState<TodoRow[]>(todos);
  useEffect(() => setItems(todos), [todos]);

  const { data: openTopics } = useTopics('open');
  // Without names, "adopted from" and @mentions on this card read "Unknown".
  const nameById = usePersonNames();
  const topicOptions = useMemo<MentionTopic[]>(
    () => (Array.isArray(openTopics) ? openTopics : []).map((t) => ({ id: t.id, name: t.name })),
    [openTopics],
  );

  const { setStatus, editDescription, deleteItem, add, setDueDate, reorderWithin } = useActionItemMutations<TodoRow>({
    setItems,
    meetingId: null, // a standalone to-do, tagged to this topic
    toRow: (item) => ({
      meetingTitle: null,
      meetingCreatedAt: null,
      sourceCount: 0,
      latestSourceAt: null,
      latestContext: null,
      ...item,
    }),
    onStatusChanged: () => void onChanged(),
    onAdded: () => void onChanged(),
  });
  const toggleHighlight = useTodoHighlight(items, setItems);

  const handleSetStatus = useCallback(
    (item: ActionItem, status: ActionItemStatus) => setStatus(item, status),
    [setStatus],
  );

  const commitEdit = async (
    item: ActionItem,
    title: string,
    dueDate: string | null,
    topic?: MentionTopic | null,
  ) => {
    if (title && title !== item.description) await editDescription(item, title);
    if (title && dueDate) await setDueDate(item, dueDate);
    if (topic && topic.id !== item.topicId) {
      try {
        await setActionItemTopic(item.id, topic.id);
      } catch (err) {
        toast.error('Could not tag the Topic', { description: errorText(err) });
      }
    }
    await onChanged();
  };

  const handleDelete = async (item: ActionItem) => {
    await deleteItem(item);
    await onChanged();
  };

  const dismiss = useCallback(
    async (ids: string[]) => {
      if (ids.length === 0) return;
      const before = items;
      setItems((prev) => prev.filter((i) => !ids.includes(i.id)));
      try {
        await clearTopicTodos(topicId, ids);
        await onChanged();
      } catch (err) {
        setItems(before);
        toast.error('Could not dismiss that', { description: errorText(err) });
      }
    },
    [items, topicId, onChanged],
  );

  // Rows follow the global order; re-sorting here is what lands an optimistic drag.
  const rows = useMemo(() => [...items].sort(bySortOrder), [items]);
  // Ticked-off rows hold their place on screen but are left out of the saved order.
  const applyOrder = useCallback(
    (next: string[]) => {
      const open = next.filter((id) => items.find((i) => i.id === id)?.status !== 'completed');
      const current = rows.filter((r) => r.status !== 'completed').map((r) => r.id);
      if (!sameOrder(open, current)) void reorderWithin(open);
    },
    [items, rows, reorderWithin],
  );

  // "Show in To-Do": optimistic, reverted on failure; the refetch settles it.
  const [shown, setShown] = useState(!hideFromTodo);
  useEffect(() => setShown(!hideFromTodo), [hideFromTodo]);
  const setShowInTodo = async (checked: boolean) => {
    setShown(checked);
    try {
      await updateTopic(topicId, { hideFromTodo: !checked });
      await onChanged();
    } catch (err) {
      console.error('Failed to change Show in To-Do:', err);
      setShown(!checked);
      toast.error('Could not change where its To-Dos show', { description: errorText(err) });
    }
  };

  const openCount = items.filter((i) => i.status === 'open').length;
  const doneIds = items.filter((i) => i.status === 'completed').map((i) => i.id);

  return (
    <Panel
      title="My action items"
      count={openCount}
      action={
        <div className="flex items-center gap-3">
          {doneIds.length > 0 ? (
            <Button size="xs" variant="ghost" onClick={() => void dismiss(doneIds)}>
              Dismiss done
            </Button>
          ) : null}
          <div className="flex items-center gap-2">
            <Switch
              id={`show-in-todo-${topicId}`}
              checked={shown}
              onCheckedChange={(v) => void setShowInTodo(v)}
            />
            <Label htmlFor={`show-in-todo-${topicId}`} className="text-xs font-normal text-muted-foreground">
              Show in To-Do
            </Label>
          </div>
        </div>
      }
    >
      <div className="flex flex-col gap-2">
        <AddActionItemRow
          placeholder="Add an action item…"
          ariaLabel="Add an action item to this Topic"
          onAdd={(description, due) => add(description, due, topicId)}
        />
        {items.length > 0 && (
          <>
            <TodoList
              className="-mx-1"
              rows={rows}
              ariaLabel="This Topic's action items"
              nameById={nameById}
              topics={topicOptions}
              showTopic={false}
              compact
              onSetStatus={handleSetStatus}
              onCommitEdit={commitEdit}
              onSetDueDate={setDueDate}
              onDelete={handleDelete}
              onOpen={(item) => router.push(`/tasks/item?id=${item.id}`)}
              onToggleHighlight={(id, color) => void toggleHighlight(id, color)}
              onDismissDone={(item) => void dismiss([item.id])}
              reorder={applyOrder}
            />
            <p className="px-2 text-caption text-muted-foreground">{todoKeysHint(true)}</p>
          </>
        )}
      </div>
    </Panel>
  );
}
