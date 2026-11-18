'use client';

/**
 * The To-Do rail of the record screen's Focus view (owner feedback, v0.18): the live
 * Meeting's Topic To-Dos, or, when it is on no Topic, your To-Dos that have no Topic.
 *
 * - Topics come from `topicsForMeeting` (open ones only: a closed Topic refuses new tags).
 *   With several, a `TopicPicker` chooses which list shows; it defaults to the first, and its
 *   "No Topic" entry shows the no-Topic list.
 * - The add field creates the To-Do on the live Meeting (`api_create_action_item`) and tags it
 *   to the Topic shown (`api_set_action_item_topic`), with the shared @mention / "(Friday)"
 *   parsing of `AddActionItemRow`. It writes no "☐ To-do:" line into the notes: this list is
 *   where the item shows. ⌘⇧T quick add is unchanged.
 * - Rows are the shared `TodoList` in its compact two-line form, as on a Topic's card.
 */

import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useRouter } from 'next/navigation';
import { toast } from 'sonner';
import { AddActionItemRow } from '@/components/ActionItems/AddActionItemRow';
import { TodoList } from '@/components/ActionItems/TodoList';
import { TopicPicker } from '@/components/ActionItems/TopicPicker';
import type { MentionTopic } from '@/components/ActionItems/MentionInput';
import { Callout } from '@/components/ui/callout';
import { PanelHeader } from '@/components/ui/panel';
import { useActionItemMutations } from '@/hooks/useActionItemMutations';
import { usePersonNames } from '@/hooks/usePersonNames';
import { useRecordTodos } from '@/hooks/useRecordTodos';
import { useTodoHighlight } from '@/hooks/useTodoHighlight';
import { useTopics } from '@/hooks/useTopics';
import { safeListen } from '@/lib/safe-listen';
import { sameOrder } from '@/lib/topic-order';
import { setActionItemTopic, topicsForMeeting } from '@/lib/topics';
import type { ActionItem, ActionItemStatus, Topic, TodoRow } from '@/types';

const errorText = (err: unknown) => (err instanceof Error ? err.message : String(err));
const bySortOrder = (a: TodoRow, b: TodoRow) => (a.sortOrder ?? Infinity) - (b.sortOrder ?? Infinity);

/** The live Meeting's open Topics; null until the first load settles. */
function useMeetingOpenTopics(meetingId: string | null): Topic[] | null {
  const [topics, setTopics] = useState<Topic[] | null>(null);
  const seq = useRef(0);
  const load = useCallback(async () => {
    if (!meetingId) return;
    const mine = ++seq.current;
    try {
      const next = await topicsForMeeting(meetingId);
      if (mine === seq.current) setTopics((Array.isArray(next) ? next : []).filter((t) => t.status === 'open'));
    } catch (err) {
      console.error('Could not load this Meeting’s Topics', err);
      if (mine === seq.current) setTopics([]);
    }
  }, [meetingId]);
  useEffect(() => {
    setTopics(null);
    void load();
  }, [load]);
  useEffect(() => safeListen('topics-updated', () => void load()), [load]);
  return topics;
}

/** `undefined` = no explicit pick yet (follow the first Topic); `null` = "No Topic". */
type Pick = string | null | undefined;

export function RecordTodoRail({ meetingId }: { meetingId: string | null }) {
  return (
    <aside
      aria-label="To-Dos"
      className="flex w-[32%] min-w-[260px] max-w-[380px] flex-shrink-0 flex-col overflow-hidden border-l border-border bg-background"
    >
      {meetingId ? (
        <RailBody key={meetingId} meetingId={meetingId} />
      ) : (
        <div className="px-5 py-4">
          <PanelHeader title="To-Dos" />
          <p className="u-meta">To-Dos for this Meeting show here once recording starts.</p>
        </div>
      )}
    </aside>
  );
}

function RailBody({ meetingId }: { meetingId: string }) {
  const router = useRouter();
  const topics = useMeetingOpenTopics(meetingId);
  const [pick, setPick] = useState<Pick>(undefined);
  const shown: Topic | null = useMemo(() => {
    if (!topics || topics.length === 0 || pick === null) return null;
    return topics.find((t) => t.id === pick) ?? topics[0];
  }, [topics, pick]);
  const shownId = shown?.id ?? null;

  const { items, setItems, loaded, error, reload } = useRecordTodos(shownId, topics !== null);

  const { data: openTopics } = useTopics('open');
  const nameById = usePersonNames();
  const mentionTopics = useMemo<MentionTopic[]>(
    () => (Array.isArray(openTopics) ? openTopics : []).map((t) => ({ id: t.id, name: t.name })),
    [openTopics],
  );
  const pickerTopics = useMemo<MentionTopic[]>(
    () => (topics ?? []).map((t) => ({ id: t.id, name: t.name })),
    [topics],
  );

  const { setStatus, editDescription, deleteItem, add, setDueDate, reorderWithin } = useActionItemMutations<TodoRow>({
    setItems,
    meetingId,
    toRow: (item) => ({
      meetingTitle: null,
      meetingCreatedAt: null,
      sourceCount: 0,
      latestSourceAt: null,
      latestContext: null,
      ...item,
    }),
    // Only rows that belong to the list on screen; an @-tag to another Topic files it there.
    appendCreated: (created) => (shownId ? created.topicId === shownId : !created.topicId),
    onAdded: () => void reload(),
  });
  const toggleHighlight = useTodoHighlight(items, setItems);

  const handleSetStatus = useCallback(
    (item: ActionItem, status: ActionItemStatus) => setStatus(item, status),
    [setStatus],
  );

  const commitEdit = async (item: ActionItem, title: string, dueDate: string | null, topic?: MentionTopic | null) => {
    if (title && title !== item.description) await editDescription(item, title);
    if (title && dueDate) await setDueDate(item, dueDate);
    if (topic && topic.id !== item.topicId) {
      try {
        await setActionItemTopic(item.id, topic.id);
      } catch (err) {
        toast.error('Could not tag the Topic', { description: errorText(err) });
      }
    }
  };

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

  const openCount = items.filter((i) => i.status === 'open').length;
  const several = (topics?.length ?? 0) > 1;
  const listName = shown ? shown.name : 'No Topic';

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex-shrink-0 px-5 pb-2 pt-4">
        <PanelHeader
          title="To-Dos"
          count={loaded ? openCount : null}
          action={
            several ? (
              <TopicPicker
                topics={pickerTopics}
                value={shown ? { id: shown.id, name: shown.name } : null}
                onChange={(t) => setPick(t ? t.id : null)}
              />
            ) : null
          }
        />
        {!several && topics !== null ? (
          <p className="u-row-meta -mt-1 mb-2 truncate">
            {shown ? `From the Topic ${shown.name}` : 'Your To-Dos with no Topic'}
          </p>
        ) : null}
        <AddActionItemRow
          placeholder="Add a To-Do…"
          ariaLabel={shown ? `Add a To-Do to this Meeting and ${listName}` : 'Add a To-Do to this Meeting'}
          topics={mentionTopics}
          onAdd={(description, due, topicId) => add(description, due, topicId ?? shownId)}
        />
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto px-4 pb-4">
        {error ? (
          <Callout tone="error" size="compact">
            {error}
          </Callout>
        ) : loaded && rows.length === 0 ? (
          <p className="u-meta px-1 pt-1">
            {shown ? `No open To-Dos on ${shown.name}.` : 'No To-Dos without a Topic.'}
          </p>
        ) : (
          <TodoList
            rows={rows}
            ariaLabel={shown ? `${shown.name} To-Dos` : 'To-Dos with no Topic'}
            nameById={nameById}
            topics={mentionTopics}
            showTopic={false}
            compact
            onSetStatus={handleSetStatus}
            onCommitEdit={commitEdit}
            onSetDueDate={setDueDate}
            onDelete={deleteItem}
            onOpen={(item) => router.push(`/tasks/item?id=${item.id}`)}
            onToggleHighlight={(id, color) => void toggleHighlight(id, color)}
            reorder={applyOrder}
          />
        )}
      </div>
    </div>
  );
}
