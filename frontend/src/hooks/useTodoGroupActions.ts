'use client';

/**
 * The To-do page's grouped-view moves (specs/0090), kept out of the page:
 *
 * - `moveToTopic`: a row dropped on another section. Optimistically re-tags it and gives
 *   it a place among the target section (the slot rule over the target's ids plus its
 *   own), then `api_set_action_item_topic` (null clears), then
 *   `api_reorder_action_items_within` with the target section's ids. A failure restores
 *   the list and says so.
 * - `reorderTopics`: a section dropped above or below another. The sections' topics take
 *   the places they hold in the one topic order (`placeInSlots`), so topics without a
 *   section keep theirs; the full order is saved, and the Topics page and sidebar follow.
 *   `topics` carries the optimistic order until the `topics-updated` refetch lands.
 */

import { useCallback, useEffect, useMemo, useState, type Dispatch, type SetStateAction } from 'react';
import { toast } from 'sonner';
import { placeInSlots, sameOrder, slotSortOrders, sortTopics } from '@/lib/topic-order';
import { reorderTopics as saveTopicOrder, setActionItemTopic } from '@/lib/topics';
import type { TodoRow, TopicSummary } from '@/types';

const errorText = (err: unknown) => (err instanceof Error ? err.message : String(err));

export function useTodoGroupActions({
  setItems,
  allTopics,
  reorderWithin,
}: {
  setItems: Dispatch<SetStateAction<TodoRow[]>>;
  allTopics: TopicSummary[];
  reorderWithin: (ids: string[]) => Promise<void>;
}) {
  const [override, setOverride] = useState<TopicSummary[] | null>(null);
  useEffect(() => setOverride(null), [allTopics]);
  const topics = useMemo(() => override ?? allTopics, [override, allTopics]);

  const reorderTopics = useCallback(
    (sectionIds: string[]) => {
      const current = sortTopics(topics).map((t) => t.id);
      const ids = placeInSlots(current, sectionIds);
      if (sameOrder(ids, current)) return;
      const rank = new Map(ids.map((id, i) => [id, i]));
      setOverride(topics.map((t) => ({ ...t, sortOrder: rank.get(t.id) ?? t.sortOrder })));
      saveTopicOrder(ids).catch((err: unknown) => {
        console.error('Failed to reorder topics:', err);
        setOverride(null);
        toast.error('Could not save the Topic order', { description: errorText(err) });
      });
    },
    [topics],
  );

  const moveToTopic = useCallback(
    async (id: string, topicId: string | null, targetIds: string[]) => {
      const topicName = topicId ? (topics.find((t) => t.id === topicId)?.name ?? null) : null;
      let snapshot: TodoRow[] = [];
      setItems((prev) => {
        snapshot = prev;
        const slots = slotSortOrders(prev, targetIds);
        return prev.map((i) => {
          const sortOrder = slots.get(i.id) ?? i.sortOrder;
          return i.id === id ? { ...i, topicId, topicName, sortOrder } : { ...i, sortOrder };
        });
      });
      try {
        await setActionItemTopic(id, topicId);
      } catch (err) {
        console.error('Failed to move the to-do:', err);
        setItems(() => snapshot);
        toast.error(topicId ? `Could not move it to ${topicName ?? 'that Topic'}` : 'Could not clear its Topic', {
          description: errorText(err),
        });
        return;
      }
      await reorderWithin(targetIds);
    },
    [topics, setItems, reorderWithin],
  );

  return { topics, reorderTopics, moveToTopic };
}
