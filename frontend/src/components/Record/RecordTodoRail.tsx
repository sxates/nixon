'use client';

/**
 * The record screen's To-Dos (owner feedback, v0.18): the live Meeting's Topic To-Dos, or,
 * when it is on no Topic, your To-Dos that have no Topic. One implementation in two places:
 * `RecordTodos` is the To-Dos tab of the Transcript view's rail, and `RecordTodoRail` is the
 * My Notes / Agenda views' right-hand sidebar (the same `RecordTodos`, with its heading row).
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
import { RECORD_STRIP_HEIGHT } from '@/lib/record-view';
import { cn } from '@/lib/utils';
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

/** My Notes / Agenda views: the right-hand sidebar, starting with its To-Dos heading row. */
export function RecordTodoRail({ meetingId }: { meetingId: string | null }) {
  return (
    <aside
      aria-label="To-Dos"
      // The Transcript view's rail width (RecordRail), so the right column doesn't change
      // size between views (owner feedback, v0.18).
      className="flex w-[40%] max-w-[440px] flex-shrink-0 flex-col overflow-hidden border-l border-border bg-background"
    >
      <RecordTodos meetingId={meetingId} />
    </aside>
  );
}

/** The heading row's box: the record screen's shared strip. */
const HEADING_STRIP = cn('flex flex-shrink-0 items-center border-b border-border px-5 py-1.5', RECORD_STRIP_HEIGHT);

/**
 * The To-Do list itself. `heading` (default on) draws the "To-Dos" heading row: a
 * RECORD_STRIP_HEIGHT strip with its rule at the bottom, so it lines up with the meeting bar
 * across the screen (owner feedback, v0.18). The Transcript rail's To-Dos tab turns it off,
 * since the tab already names it.
 */
export function RecordTodos({ meetingId, heading = true }: { meetingId: string | null; heading?: boolean }) {
  if (!meetingId) {
    return (
      <div>
        {heading && (
          <div className={HEADING_STRIP}>
            <PanelHeader title="To-Dos" className="mb-0 flex-1" />
          </div>
        )}
        <p className="u-meta px-5 py-4">To-Dos for this Meeting show here once recording starts.</p>
      </div>
    );
  }
  return <RailBody key={meetingId} meetingId={meetingId} heading={heading} />;
}

function RailBody({ meetingId, heading }: { meetingId: string; heading: boolean }) {
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
  const picker = several ? (
    <TopicPicker
      topics={pickerTopics}
      value={shown ? { id: shown.id, name: shown.name } : null}
      onChange={(t) => setPick(t ? t.id : null)}
    />
  ) : null;

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      {heading && (
        <div className={HEADING_STRIP}>
          <PanelHeader
            title="To-Dos"
            count={loaded ? openCount : null}
            action={picker}
            className="mb-0 flex-1"
          />
        </div>
      )}
      <div className="flex-shrink-0 px-5 pb-2 pt-3">
        {!heading && several && <PanelHeader action={picker} />}
        {!several && topics !== null ? (
          <p className="u-row-meta mb-2 truncate">
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
