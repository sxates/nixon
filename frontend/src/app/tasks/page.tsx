'use client';

/**
 * To-do (specs/0088, replacing the meeting-grouped task hub of specs/0034/0038) — one
 * hand-ordered list of your commitments, a suggestion Inbox, and Done history, as three
 * tabs. The selected tab syncs to `?tab=todo|inbox|done` so a link or a screenshot can
 * address each one.
 *
 *   - **To-do**: quick-add on top (a trailing `(Friday)` becomes the due date), then the
 *     drag-ordered list (persisted via `api_reorder_action_items`). No grouping, sorting
 *     or person filter — the order is yours.
 *   - **Inbox**: items the extractor suggested. Add to my list / Dismiss / Assign…; the
 *     header menu dismisses old suggestions in bulk (with an Undo toast) and toggles the
 *     dismissed list, where each row can be restored.
 *   - **Done**: completed items by week; the checkbox reopens one.
 *
 * Lists come from `useTodoViews`, which refetches on `action-items-updated`; mutations
 * here also call `reload()` so the screen settles without waiting for the event.
 */

import { Suspense, useCallback, useEffect, useMemo, useState } from 'react';
import { useRouter, useSearchParams } from 'next/navigation';
import { invoke } from '@tauri-apps/api/core';
import { toast } from 'sonner';
import { Callout } from '@/components/ui/callout';
import { EmptyState, PageLoading } from '@/components/ui/empty-state';
import { MenuButton } from '@/components/ui/menu-button';
import { PageHeader } from '@/components/ui/page-header';
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/components/ui/tabs';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu';
import { AddActionItemRow } from '@/components/ActionItems/AddActionItemRow';
import type { AssigneeCandidate } from '@/components/ActionItems/AssigneePicker';
import { DoneList } from '@/components/ActionItems/DoneList';
import { InboxRow } from '@/components/ActionItems/InboxRow';
import { TodoList, todoKeysHint } from '@/components/ActionItems/TodoList';
import { useTodoHighlight } from '@/hooks/useTodoHighlight';
import {
  FOCUS_QUICK_ADD_EVENT,
  QUICK_ADD_LABEL,
  focusQuickAdd,
} from '@/components/QuickAdd/QuickAddShortcut';
import { useActionItemMutations } from '@/hooks/useActionItemMutations';
import { useTopics } from '@/hooks/useTopics';
import { setActionItemTopic } from '@/lib/topics';
import type { MentionTopic } from '@/components/ActionItems/MentionInput';
import { useTodoViews } from '@/hooks/useTodoViews';
import {
  updateAssigneeArgs,
  type AssigneeSelection,
} from '@/lib/action-items';
import { PAGE_CONTENT_WIDTH, PAGE_GUTTER } from '@/lib/page-width';
import { cn } from '@/lib/utils';
import type { ActionItem, ActionItemStatus, Person, TodoRow } from '@/types';

type TabValue = 'todo' | 'inbox' | 'done';
const TAB_VALUES: TabValue[] = ['todo', 'inbox', 'done'];
const isTab = (v: string | null): v is TabValue => !!v && (TAB_VALUES as string[]).includes(v);

const errorText = (err: unknown) => (err instanceof Error ? err.message : String(err));

function TasksPageContent() {
  const router = useRouter();
  const searchParams = useSearchParams();

  const paramTab = searchParams.get('tab');
  const [tab, setTab] = useState<TabValue>(isTab(paramTab) ? paramTab : 'todo');
  useEffect(() => {
    if (isTab(paramTab)) setTab(paramTab);
  }, [paramTab]);
  const changeTab = (next: string) => {
    if (!isTab(next)) return;
    setTab(next);
    if (next !== 'inbox') setShowDismissed(false);
    router.replace(`/tasks?tab=${next}`);
  };

  const [showDismissed, setShowDismissed] = useState(false);
  const views = useTodoViews({ includeDismissed: showDismissed });
  const { reload } = views;

  const { data: openTopics } = useTopics('open');
  const topicOptions = useMemo<MentionTopic[]>(
    () => (openTopics ?? []).map((t) => ({ id: t.id, name: t.name })),
    [openTopics],
  );
  const [people, setPeople] = useState<Person[]>([]);
  useEffect(() => {
    void (async () => {
      try {
        const result = await invoke<Person[]>('api_list_people');
        setPeople(Array.isArray(result) ? result : []);
      } catch (err) {
        console.error('Failed to load people:', err);
      }
    })();
  }, []);
  const nameById = useMemo(() => new Map(people.map((p) => [p.id, p.displayName])), [people]);
  const candidates = useMemo<AssigneeCandidate[]>(
    () => people.map((p) => ({ personId: p.id, displayName: p.displayName, email: p.email })),
    [people],
  );

  // The To-do list is mirrored locally so checkbox / reorder / edit can be optimistic.
  const [items, setItems] = useState<TodoRow[]>([]);
  useEffect(() => setItems(views.list), [views.list]);

  const {
    setStatus,
    editDescription,
    assign,
    deleteItem,
    add,
    reorder,
    bulkSetStatus,
    triage,
    dismissOlderThan,
    setDueDate,
  } = useActionItemMutations<TodoRow>({
    setItems,
    meetingId: null, // quick-add creates standalone to-dos
    toRow: (item) => ({
      meetingTitle: null,
      meetingCreatedAt: null,
      sourceCount: 0,
      latestSourceAt: null,
      latestContext: null,
      ...item,
    }),
    // The legacy commands emit no event: re-read so the lists and badge stay correct.
    onStatusChanged: () => void reload(),
    onAssigned: () => reload(),
    onAdded: (_created, tagFailed) => {
      if (!tagFailed) toast.success('To-do added');
      void reload();
    },
  });

  const handleDelete = async (item: ActionItem) => {
    await deleteItem(item);
    void reload();
  };

  const openItem = useCallback(
    (item: ActionItem) => router.push(`/tasks/item?id=${item.id}`),
    [router],
  );

  // ---- To-do drag order ----
  // Hand order: `sortOrder` ascending, unordered rows after, ties keep backend order (the
  // sort is stable). Re-sorting here is what makes an optimistic drag land on screen.
  const openItems = useMemo(
    () =>
      items
        .filter((i) => i.status === 'open')
        .sort((a, b) => (a.sortOrder ?? Infinity) - (b.sortOrder ?? Infinity) || 0),
    [items],
  );
  // Ticked off on this visit: the row stays in place, struck through, so a mis-click can be
  // unticked. It moves to Done once you leave the page (this state goes with it).
  const [lingering, setLingering] = useState<Map<string, TodoRow>>(new Map());
  const listRows = useMemo(() => {
    const shown = openItems.filter((i) => !lingering.has(i.id));
    return [...shown, ...lingering.values()].sort(
      (a, b) => (a.sortOrder ?? Infinity) - (b.sortOrder ?? Infinity) || 0,
    );
  }, [openItems, lingering]);
  const setListStatus = useCallback(
    (item: ActionItem, status: ActionItemStatus) => {
      setLingering((prev) => {
        const next = new Map(prev);
        if (status === 'completed') {
          const row = items.find((i) => i.id === item.id) ?? (item as TodoRow);
          next.set(item.id, { ...row, status: 'completed' });
        } else next.delete(item.id);
        return next;
      });
      return setStatus(item, status);
    },
    [items, setStatus],
  );
  // Ticked-off rows hold their place on screen but are no longer part of the saved order.
  const applyOrder = useCallback(
    (next: string[]) => void reorder(next.filter((id) => !lingering.has(id))),
    [reorder, lingering],
  );

  const toggleHighlight = useTodoHighlight(items, setItems);

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
        void reload();
      } catch (err) {
        toast.error('Could not tag the topic', { description: errorText(err) });
      }
    }
  };

  // ⌘T (QuickAddShortcut): focus the new-item field, on arrival (?add=1) or in place.
  const wantsAdd = searchParams.get('add') === '1';
  useEffect(() => {
    if (!wantsAdd || !views.loaded) return;
    if (focusQuickAdd()) router.replace('/tasks?tab=todo');
  }, [wantsAdd, views.loaded, router]);
  useEffect(() => {
    const onFocus = () => {
      changeTab('todo');
      requestAnimationFrame(() => focusQuickAdd());
    };
    window.addEventListener(FOCUS_QUICK_ADD_EVENT, onFocus);
    return () => window.removeEventListener(FOCUS_QUICK_ADD_EVENT, onFocus);
  });

  // ---- Inbox triage ----
  const runTriage = async (item: TodoRow, action: 'add_to_list' | 'dismiss' | 'restore') => {
    try {
      await triage(item.id, action);
    } catch (err) {
      console.error('Failed to triage action item:', err);
      toast.error('Could not update the item', { description: errorText(err) });
    }
    await reload();
  };

  const assignSuggestion = async (item: TodoRow, selection: AssigneeSelection) => {
    // "Me" is the same as adding it to my list; anyone else takes it off my plate.
    if (selection.kind === 'me') return runTriage(item, 'add_to_list');
    try {
      await invoke('api_update_action_item', updateAssigneeArgs(item.id, selection));
    } catch (err) {
      console.error('Failed to assign action item:', err);
      toast.error('Could not update the assignee', { description: errorText(err) });
    }
    await reload();
  };

  const handleDismissOlder = async (days: 14 | 30 | 60) => {
    try {
      const ids = await dismissOlderThan(days);
      if (ids.length === 0) {
        toast('Nothing to dismiss', {
          description: `No suggestions older than ${days} days.`,
        });
        return;
      }
      toast.success(`Dismissed ${ids.length} ${ids.length === 1 ? 'suggestion' : 'suggestions'}`, {
        action: {
          label: 'Undo',
          onClick: () => {
            void (async () => {
              try {
                await bulkSetStatus({ mode: 'ids', ids }, 'open');
              } catch (err) {
                toast.error('Could not undo', { description: errorText(err) });
              }
              await reload();
            })();
          },
        },
      });
    } catch (err) {
      toast.error('Could not dismiss those suggestions', { description: errorText(err) });
    }
    await reload();
  };

  const inboxRows = showDismissed ? views.dismissed : views.inbox;
  const doneRows = useMemo(() => views.done.filter((r) => r.status === 'completed'), [views.done]);

  const noun = (n: number, one: string, many: string) => `${n} ${n === 1 ? one : many}`;
  const subtitle =
    tab === 'inbox'
      ? inboxRows.length > 0
        ? showDismissed
          ? `${noun(inboxRows.length, 'dismissed suggestion', 'dismissed suggestions')}`
          : `${noun(inboxRows.length, 'suggestion', 'suggestions')} to review`
        : 'Suggestions from your meetings'
      : tab === 'done'
        ? doneRows.length > 0
          ? `${doneRows.length} completed`
          : 'What you finish lands here'
        : openItems.length > 0
          ? `${noun(openItems.length, 'item', 'items')} on your list`
          : 'Your commitments, in the order you choose';

  return (
    // A short opacity fade, no travel (docs/design-system.md §Motion).
    <div className="flex h-page animate-fade-in flex-col bg-background">
      <PageHeader
        width="page"
        title="To-do"
        subtitle={subtitle}
        actions={
          // The slot exists on every tab so the header doesn't jump when the menu appears.
          <div className="size-7">
            {tab === 'inbox' && (
              <DropdownMenu>
                <DropdownMenuTrigger asChild>
                  <MenuButton label="Inbox options" />
                </DropdownMenuTrigger>
                <DropdownMenuContent align="end">
                  {([14, 30, 60] as const).map((days) => (
                    <DropdownMenuItem key={days} onSelect={() => void handleDismissOlder(days)}>
                      Dismiss suggestions older than {days} days
                    </DropdownMenuItem>
                  ))}
                  <DropdownMenuSeparator />
                  <DropdownMenuItem onSelect={() => setShowDismissed((v) => !v)}>
                    {showDismissed ? 'Hide dismissed' : 'Show dismissed'}
                  </DropdownMenuItem>
                </DropdownMenuContent>
              </DropdownMenu>
            )}
          </div>
        }
      />

      <Tabs value={tab} onValueChange={changeTab} className="flex min-h-0 flex-1 flex-col">
        <div className={cn('flex-shrink-0', PAGE_GUTTER)}>
          <TabsList className={cn('mx-auto w-full', PAGE_CONTENT_WIDTH)}>
            <TabsTrigger value="todo">To-do</TabsTrigger>
            <TabsTrigger value="inbox" badge={views.inboxCount > 0 ? views.inboxCount : undefined}>
              Inbox
            </TabsTrigger>
            <TabsTrigger value="done">Done</TabsTrigger>
          </TabsList>
        </div>

        <div className={cn('flex-1 overflow-y-auto pb-12', PAGE_GUTTER)}>
          <div className={cn('mx-auto w-full', PAGE_CONTENT_WIDTH)}>
            {views.error && (
              <Callout tone="error" className="mt-4">
                {views.error}
              </Callout>
            )}
            {!views.loaded ? (
              <PageLoading fill="pane" label="Loading your to-do list…" />
            ) : (
              <>
                <TabsContent value="todo" className="mt-4">
                  <AddActionItemRow
                    placeholder="New item — add a to-do…  (⌘⇧T from anywhere)"
                    ariaLabel={QUICK_ADD_LABEL}
                    onAdd={add}
                    topics={topicOptions}
                  />
                  {listRows.length === 0 ? (
                    <EmptyState
                      className="h-[40vh]"
                      title="Nothing on your list"
                      body="Add a to-do above, or move suggestions over from your Inbox."
                    />
                  ) : (
                    <>
                      <TodoList
                        className="mt-3"
                        rows={listRows}
                        ariaLabel="To-do list"
                        nameById={nameById}
                        topics={topicOptions}
                        onSetStatus={setListStatus}
                        onCommitEdit={commitEdit}
                        onSetDueDate={setDueDate}
                        onDelete={handleDelete}
                        onOpen={openItem}
                        onOpenTopic={(topicId) => router.push(`/topics/topic?id=${encodeURIComponent(topicId)}`)}
                        onToggleHighlight={(id, color) => void toggleHighlight(id, color)}
                        reorder={applyOrder}
                      />
                      <p className="mt-4 px-3 text-caption text-muted-foreground">{todoKeysHint(true)}</p>
                    </>
                  )}
                </TabsContent>

                <TabsContent value="inbox" className="mt-4">
                  {inboxRows.length === 0 ? (
                    <EmptyState
                      className="h-[40vh]"
                      title={showDismissed ? 'Nothing dismissed' : 'Nothing waiting'}
                      body={
                        showDismissed
                          ? 'Suggestions you dismiss show up here, so you can bring them back.'
                          : 'New suggestions from your meetings land here for you to triage.'
                      }
                    />
                  ) : (
                    <div className="flex flex-col gap-0.5 px-0.5">
                      {inboxRows.map((item) => (
                        <InboxRow
                          key={item.id}
                          item={item}
                          mode={showDismissed ? 'dismissed' : 'inbox'}
                          nameById={nameById}
                          candidates={candidates}
                          onAdd={(row) => runTriage(row, 'add_to_list')}
                          onDismiss={(row) => runTriage(row, 'dismiss')}
                          onAssign={assignSuggestion}
                          onRestore={(row) => runTriage(row, 'restore')}
                          onOpen={openItem}
                        />
                      ))}
                    </div>
                  )}
                </TabsContent>

                <TabsContent value="done" className="mt-4">
                  {doneRows.length === 0 ? (
                    <EmptyState
                      className="h-[40vh]"
                      title="Nothing completed yet"
                      body="Items you tick off land here."
                    />
                  ) : (
                    <DoneList
                      rows={doneRows}
                      nameById={nameById}
                      candidates={candidates}
                      onSetStatus={setStatus}
                      onEditDescription={editDescription}
                      onAssign={assign}
                      onDelete={handleDelete}
                      onOpen={openItem}
                    />
                  )}
                </TabsContent>
              </>
            )}
          </div>
        </div>
      </Tabs>
    </div>
  );
}

export default function TasksPage() {
  // useSearchParams requires a Suspense boundary (same pattern as the Ask page).
  return (
    <Suspense fallback={<PageLoading className="bg-background" />}>
      <TasksPageContent />
    </Suspense>
  );
}
