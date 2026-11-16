/**
 * The toast for a to-do that lands on a topic kept off the To-do list (specs/0090): the
 * item leaves (or never joins) the list, so say where it went and offer the way there —
 * the 0088 lesson that a silently vanishing item reads as a lost one.
 */

import { toast } from 'sonner';
import type { TopicSummary } from '@/types';

export function keptOffTopic(
  topics: readonly TopicSummary[] | null | undefined,
  topicId: string | null | undefined,
): TopicSummary | null {
  if (!topicId) return null;
  return (topics ?? []).find((t) => t.id === topicId && t.hideFromTodo) ?? null;
}

export function toastKeptOff(
  kind: 'added' | 'moved',
  topic: { id: string; name: string },
  openTopic: (id: string) => void,
  list = 'this list',
): void {
  toast.success(`${kind === 'added' ? 'Added' : 'Moved'} to ${topic.name}, which is kept off ${list}`, {
    action: { label: 'Open Topic', onClick: () => openTopic(topic.id) },
  });
}
