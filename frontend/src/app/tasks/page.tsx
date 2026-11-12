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
import { ActionItemRow } from '@/components/ActionItems/ActionItemRow';
import { AddActionItemRow } from '@/components/ActionItems/AddActionItemRow';
import type { AssigneeCandidate } from '@/components/ActionItems/AssigneePicker';
import { DoneList } from '@/components/ActionItems/DoneList';
import { InboxRow } from '@/components/ActionItems/InboxRow';
import { useActionItemMutations } from '@/hooks/useActionItemMutations';
import { useTodoViews } from '@/hooks/useTodoViews';
import {
  adoptedFromName,
  assigneeLabel,
  reorderIds,
  updateAssigneeArgs,
  type AssigneeSelection,
} from '@/lib/action-items';
import { PAGE_CONTENT_WIDTH, PAGE_GUTTER } from '@/lib/page-width';
import { cn } from '@/lib/utils';
import type { ActionItem, Person, TodoRow } from '@/types';

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
    onAdded: () => {
      toast.success('To-do added');
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
  const [draggingId, setDraggingId] = useState<string | null>(null);
  const [overId, setOverId] = useState<string | null>(null);
  const [armedId, setArmedId] = useState<string | null>(null);
  const resetDrag = useCallback(() => {
    setDraggingId(null);
    setOverId(null);
    setArmedId(null);
  }, []);
  // Hand order: `sortOrder` ascending, unordered rows after, ties keep backend order (the
  // sort is stable). Re-sorting here is what makes an optimistic drag land on screen.
  const openItems = useMemo(
    () =>
      items
        .filter((i) => i.status === 'open')
        .sort((a, b) => (a.sortOrder ?? Infinity) - (b.sortOrder ?? Infinity) || 0),
    [items],
  );
  const handleDrop = (targetId: string) => {
    if (draggingId && draggingId !== targetId) {
      void reorder(
        reorderIds(
          openItems.map((i) => i.id),
          draggingId,
          targetId,
        ),
      );
    }
    resetDrag();
  };

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
                  <AddActionItemRow placeholder="New item — add a to-do…" onAdd={add} />
                  {openItems.length === 0 ? (
                    <EmptyState
                      className="h-[40vh]"
                      title="Nothing on your list"
                      body="Add a to-do above, or move suggestions over from your Inbox."
                    />
                  ) : (
                    <div className="mt-4 flex flex-col gap-0.5 px-1">
                      {openItems.map((item) => (
                        <div
                          key={item.id}
                          draggable={armedId === item.id}
                          onDragStart={(e) => {
                            setDraggingId(item.id);
                            if (e.dataTransfer) e.dataTransfer.effectAllowed = 'move';
                          }}
                          onDragOver={(e) => {
                            if (draggingId) {
                              e.preventDefault();
                              setOverId(item.id);
                            }
                          }}
                          onDrop={(e) => {
                            e.preventDefault();
                            handleDrop(item.id);
                          }}
                          onDragEnd={resetDrag}
                          className={cn(
                            'rounded-lg transition-shadow',
                            draggingId === item.id && 'opacity-50',
                            overId === item.id &&
                              draggingId &&
                              draggingId !== item.id &&
                              'ring-2 ring-brand-edge',
                          )}
                        >
                          <ActionItemRow
                            item={item}
                            assigneeName={assigneeLabel(item, nameById)}
                            candidates={candidates}
                            nameById={nameById}
                            sourceCount={item.sourceCount}
                            onSetStatus={setStatus}
                            onEditDescription={editDescription}
                            onAssign={assign}
                            onDelete={handleDelete}
                            onOpen={openItem}
                            hideAssignee
                            adoptedFrom={adoptedFromName(item, nameById)}
                            onSetDueDate={setDueDate}
                            dragHandle={
                              <IconButton
                                variant="ghost"
                                label="Drag to reorder"
                                onMouseDown={() => setArmedId(item.id)}
                                onMouseUp={() => setArmedId(null)}
                                icon={<DeckIcon icon={GripVertical} size={14} />}
                                // A narrow grip in the row's gutter, not a 28px key: it shows
                                // on row hover and reads as a handle, not a button.
                                className="mt-0.5 h-[18px] w-4 cursor-grab text-muted-foreground/40 opacity-0 transition-opacity hover:bg-transparent hover:text-muted-foreground focus-visible:opacity-100 group-hover:opacity-100 active:cursor-grabbing"
                              />
                            }
                          />
                        </div>
                      ))}
                    </div>
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
