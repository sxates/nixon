'use client';

/**
 * My open action items (specs/0089 §Action items integration): the listed, open to-dos
 * tagged to this topic, in the To-do order the backend returns them. Checking one completes
 * it through the same command the To-do list uses, so it is done everywhere. The quick-add
 * row creates a standalone to-do and then tags it with this topic.
 */

import { useCallback, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { toast } from 'sonner';
import { AddActionItemRow } from '@/components/ActionItems/AddActionItemRow';
import { Checkbox } from '@/components/ui/checkbox';
import { Panel } from '@/components/ui/panel';
import { titleSegments } from '@/lib/todo-parse';
import { setActionItemTopic } from '@/lib/topics';
import type { ActionItem, TodoRow } from '@/types';

const NO_NAMES = new Map<string, string>();
const errorText = (err: unknown) => (err instanceof Error ? err.message : String(err));

interface TopicTodosCardProps {
  topicId: string;
  todos: TodoRow[];
  /** Refetch the topic after a change. */
  onChanged: () => void | Promise<void>;
}

export function TopicTodosCard({ topicId, todos, onChanged }: TopicTodosCardProps) {
  // Optimistically hide a checked item until the refetch lands (or put it back on failure).
  const [done, setDone] = useState<Set<string>>(new Set());

  const complete = useCallback(
    async (item: TodoRow) => {
      setDone((prev) => new Set(prev).add(item.id));
      try {
        await invoke<ActionItem>('api_set_action_item_status', { id: item.id, status: 'completed' });
        await onChanged();
      } catch (err) {
        console.error('Failed to complete the to-do:', err);
        toast.error('Could not update the item', { description: errorText(err) });
        setDone((prev) => {
          const next = new Set(prev);
          next.delete(item.id);
          return next;
        });
      }
    },
    [onChanged],
  );

  const add = useCallback(
    async (description: string, dueDate?: string) => {
      let created: ActionItem;
      try {
        created = await invoke<ActionItem>('api_create_action_item', {
          meetingId: null,
          description,
          ...(dueDate ? { dueDate } : {}),
        });
      } catch (err) {
        console.error('Failed to add the to-do:', err);
        toast.error('Could not add the item', { description: errorText(err) });
        return false;
      }
      // The item exists now: a refused tag must not make the owner retry (a duplicate).
      try {
        await setActionItemTopic(created.id, topicId);
      } catch (err) {
        toast.error('Added, but not to the topic', { description: errorText(err) });
      }
      await onChanged();
      return true;
    },
    [topicId, onChanged],
  );

  const open = todos.filter((t) => t.status === 'open' && !done.has(t.id));

  return (
    <Panel title="My open action items" count={open.length}>
      <div className="flex flex-col gap-2">
        <AddActionItemRow
          placeholder="Add an action item…"
          ariaLabel="Add an action item to this topic"
          onAdd={add}
        />
        {open.length > 0 && (
          <ul className="flex flex-col gap-1.5">
            {open.map((item) => (
              <li key={item.id} className="flex items-start gap-3 text-sm leading-5">
                <Checkbox
                  checked={false}
                  aria-label={`Mark "${item.description}" as completed`}
                  onCheckedChange={() => void complete(item)}
                  className="mt-0.5"
                />
                <span className="min-w-0 text-foreground">
                  {titleSegments(item.description, NO_NAMES).map((seg, i) =>
                    seg.kind === 'person' ? (
                      <span key={i} className="font-medium">
                        @{seg.name}
                      </span>
                    ) : (
                      <span key={i}>{seg.text}</span>
                    ),
                  )}
                </span>
              </li>
            ))}
          </ul>
        )}
      </div>
    </Panel>
  );
}
