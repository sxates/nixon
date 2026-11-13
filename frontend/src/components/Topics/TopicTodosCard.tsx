'use client';

/**
 * My action items (specs/0089 §Action items integration): the to-dos tagged to this topic,
 * as the same list the To-do page uses — quick-add on top, then ↑↓ select, Space edit, ⌘D
 * done, ⌘4 / ⌘5 highlight. The order follows your To-do list, so there is no reordering
 * here. A ticked-off item stays on the card, struck through, until you dismiss it (its ✕,
 * or "Dismiss done" for all of them); it is done everywhere the moment you tick it.
 */

import { useCallback, useEffect, useMemo, useState } from 'react';
import { useRouter } from 'next/navigation';
import { toast } from 'sonner';
import { AddActionItemRow } from '@/components/ActionItems/AddActionItemRow';
import { TodoList, todoKeysHint } from '@/components/ActionItems/TodoList';
import type { MentionTopic } from '@/components/ActionItems/MentionInput';
import { Button } from '@/components/ui/button';
import { Panel } from '@/components/ui/panel';
import { useActionItemMutations } from '@/hooks/useActionItemMutations';
import { useTodoHighlight } from '@/hooks/useTodoHighlight';
import { useTopics } from '@/hooks/useTopics';
import { clearTopicTodos, setActionItemTopic } from '@/lib/topics';
import type { ActionItem, ActionItemStatus, TodoRow } from '@/types';

const NO_NAMES = new Map<string, string>();
const errorText = (err: unknown) => (err instanceof Error ? err.message : String(err));

interface TopicTodosCardProps {
  topicId: string;
  todos: TodoRow[];
  /** Refetch the topic after a change. */
  onChanged: () => void | Promise<void>;
}

export function TopicTodosCard({ topicId, todos, onChanged }: TopicTodosCardProps) {
  const router = useRouter();
  // Mirrored locally so ticking, editing and highlighting are optimistic.
  const [items, setItems] = useState<TodoRow[]>(todos);
  useEffect(() => setItems(todos), [todos]);

  const { data: openTopics } = useTopics('open');
  const topicOptions = useMemo<MentionTopic[]>(
    () => (Array.isArray(openTopics) ? openTopics : []).map((t) => ({ id: t.id, name: t.name })),
    [openTopics],
  );

  const { setStatus, editDescription, deleteItem, add, setDueDate } = useActionItemMutations<TodoRow>({
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
        toast.error('Could not tag the topic', { description: errorText(err) });
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

  const openCount = items.filter((i) => i.status === 'open').length;
  const doneIds = items.filter((i) => i.status === 'completed').map((i) => i.id);

  return (
    <Panel
      title="My action items"
      count={openCount}
      action={
        doneIds.length > 0 ? (
          <Button size="xs" variant="ghost" onClick={() => void dismiss(doneIds)}>
            Dismiss done
          </Button>
        ) : null
      }
    >
      <div className="flex flex-col gap-2">
        <AddActionItemRow
          placeholder="Add an action item…"
          ariaLabel="Add an action item to this topic"
          onAdd={(description, due) => add(description, due, topicId)}
        />
        {items.length > 0 && (
          <>
            <TodoList
              className="-mx-1"
              rows={items}
              ariaLabel="This topic's action items"
              nameById={NO_NAMES}
              topics={topicOptions}
              showTopic={false}
              onSetStatus={handleSetStatus}
              onCommitEdit={commitEdit}
              onSetDueDate={setDueDate}
              onDelete={handleDelete}
              onOpen={(item) => router.push(`/tasks/item?id=${item.id}`)}
              onToggleHighlight={(id, color) => void toggleHighlight(id, color)}
              onDismissDone={(item) => void dismiss([item.id])}
            />
            <p className="px-2 text-caption text-muted-foreground">{todoKeysHint(false)}</p>
          </>
        )}
      </div>
    </Panel>
  );
}
