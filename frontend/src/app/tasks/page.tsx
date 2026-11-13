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

import { Suspense, useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useRouter, useSearchParams } from 'next/navigation';
import { invoke } from '@tauri-apps/api/core';
import { toast } from 'sonner';
import { Callout } from '@/components/ui/callout';
import { DeckIcon } from '@/components/ui/deck-icon';
import { EmptyState, PageLoading } from '@/components/ui/empty-state';
import { IconButton } from '@/components/ui/icon-button';
import { GripVertical } from '@/components/ui/icons';
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
import { TodoListRow, todoGrid, todoRowDomId } from '@/components/ActionItems/TodoListRow';
import { REORDER_ATTR, useDragReorder } from '@/hooks/useDragReorder';
import { moveToIndex, useTodoListKeys, type Highlight } from '@/hooks/useTodoListKeys';
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
  adoptedFromName,
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
  const toggleDone = useCallback(
    (id: string) => {
      const row = listRows.find((r) => r.id === id);
      if (row) void setListStatus(row, lingering.has(id) ? 'open' : 'completed');
    },
    [listRows, lingering, setListStatus],
  );

  const openIds = useMemo(() => listRows.map((i) => i.id), [listRows]);
  // Ticked-off rows hold their place on screen but are no longer part of the saved order.
  const applyOrder = useCallback(
    (next: string[]) => void reorder(next.filter((id) => !lingering.has(id))),
    [reorder, lingering],
  );
  const { draggingId, overId, gripProps } = useDragReorder(openIds, applyOrder);

  // ---- To-do selection, inline edit, highlight (keyboard model in useTodoListKeys) ----
  const listRef = useRef<HTMLDivElement>(null);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [editingId, setEditingId] = useState<string | null>(null);
  // The Topic column only when some row in this view is tagged (specs/0089). While an inline
  // edit is open the layout holds still, so a re-tag landing mid-edit can't shift the field.
  const anyTagged = listRows.some((i) => !!i.topicId);
  const shownTopicColumn = useRef(anyTagged);
  if (editingId === null || anyTagged) shownTopicColumn.current = anyTagged;
  const showTopic = shownTopicColumn.current;
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
  // A row that leaves the list (completed, dismissed) takes the selection with it.
  useEffect(() => {
    if (selectedId && !openIds.includes(selectedId)) setSelectedId(null);
  }, [openIds, selectedId]);

  const toggleHighlight = useCallback(
    async (id: string, color: Highlight) => {
      const current = items.find((i) => i.id === id)?.highlight ?? null;
      const next = current === color ? null : color;
      setItems((prev) => prev.map((i) => (i.id === id ? { ...i, highlight: next } : i)));
      try {
        await invoke('api_set_action_item_highlight', { id, highlight: next });
      } catch (err) {
        setItems((prev) => prev.map((i) => (i.id === id ? { ...i, highlight: current } : i)));
        toast.error('Could not change the highlight', { description: errorText(err) });
      }
    },
    [items],
  );

  const onListKeyDown = useTodoListKeys({
    ids: openIds,
    selectedId,
    editing: editingId !== null,
    select: selectRow,
    startEdit: setEditingId,
    reorder: applyOrder,
    toggleHighlight: (id, color) => void toggleHighlight(id, color),
    toggleDone,
  });

  const commitEdit = async (
    item: ActionItem,
    title: string,
    dueDate: string | null,
    topic?: MentionTopic | null,
  ) => {
    endEdit();
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
                    <div
                      ref={listRef}
                      className="group/list mt-3 px-1 focus:outline-none"
                      role="listbox"
                      aria-label="To-do list"
                      tabIndex={0}
                      aria-activedescendant={selectedId ? todoRowDomId(selectedId) : undefined}
                      onKeyDown={onListKeyDown}
                    >
                      <div className={cn(todoGrid(showTopic), 'px-2 pb-1')} aria-hidden>
                        <span />
                        <span className="u-section-label-sm">Item</span>
                        {showTopic ? <span className="u-section-label-sm">Topic</span> : null}
                        <span className="u-section-label-sm">Due</span>
                      </div>
                      {listRows.map((item) => (
                        <div
                          key={item.id}
                          role="presentation"
                          {...{ [REORDER_ATTR]: item.id }}
                          className={cn(
                            'rounded-lg transition-shadow',
                            draggingId === item.id && 'opacity-50',
                            overId === item.id &&
                              draggingId &&
                              draggingId !== item.id &&
                              'ring-2 ring-brand-edge',
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
                            onCommitEdit={(row, title, due, topic) => void commitEdit(row, title, due, topic)}
                            topics={topicOptions}
                            showTopic={showTopic}
                            onOpenTopic={(topicId) => router.push(`/topics/topic?id=${encodeURIComponent(topicId)}`)}
                            onCancelEdit={endEdit}
                            onToggleHighlight={(row, color) => void toggleHighlight(row.id, color)}
                            onMoveToTop={(row) => applyOrder(moveToIndex(openIds, row.id, 0))}
                            onOpen={openItem}
                            onSetStatus={setListStatus}
                            onSetDueDate={setDueDate}
                            onDelete={handleDelete}
                            dragHandle={
                              <IconButton
                                variant="ghost"
                                label="Drag to reorder"
                                {...gripProps(item.id)}
                                icon={<DeckIcon icon={GripVertical} size={14} />}
                                // A narrow grip in the row's gutter, not a 28px key: it shows
                                // on row hover and reads as a handle, not a button.
                                className={cn(
                                  'h-[18px] w-4 cursor-grab touch-none text-muted-foreground/40 opacity-0 transition-opacity hover:bg-transparent hover:text-muted-foreground focus-visible:opacity-100 group-hover:opacity-100 active:cursor-grabbing',
                                  draggingId === item.id && 'opacity-100',
                                )}
                              />
                            }
                          />
                        </div>
                      ))}
                    </div>
                    <p className="mt-4 px-3 text-caption text-muted-foreground">
                      ↑↓ select · Space edit · ⌘D done · ⌘↑↓ move · ⌘1–3 move to 1st–3rd · ⌘4 / ⌘5 highlight
                    </p>
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
