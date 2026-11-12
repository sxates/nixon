'use client';

/**
 * Task hub (specs/0034, extended by specs/0038 WS1) — every action item across meetings
 * on one page.
 *
 * Filter bar: a status segmented control (Open default / Completed / Dismissed), a
 * top-level VIEW segmented control (Mine / Everyone — WS1.c) with a "By person" filter menu
 * beside it, defaulting to **Mine + Open** (the owner's primary workflow), a SORT control (Manual / Due date /
 * Meeting date — WS1.b, Meeting is the default), and a bulk "Dismiss…" menu in the Open
 * view (WS1.e). Filtering happens server-side via `api_list_action_items`.
 *
 * Ordering:
 *   - **Meeting** (default): client-side grouping by source meeting (`groupActionItems`),
 *     newest meeting first, standalone to-dos in a trailing "Unattached" group; meeting
 *     headers deep-link to `/meeting-details?id=…`.
 *   - **Manual / Due**: a flat list (`sortActionItems`) — the source meeting becomes a
 *     deep-link chip on the row instead of a group header. Manual rows are drag-reorderable
 *     (persisted via `api_reorder_action_items`). Rows are compact (single line, expanding
 *     on hover/focus).
 *
 * "New item" creates a standalone manual to-do (`api_create_action_item` with no meeting).
 * Rows are the shared `ActionItemRow`; mutations come from the shared
 * `useActionItemMutations` hook (optimistic, per-item error reverts). A status change that
 * moves an item out of the current filter drops it locally rather than re-querying.
 */

import { useCallback, useEffect, useMemo, useState } from 'react';
import { useRouter } from 'next/navigation';
import { invoke } from '@tauri-apps/api/core';
import { toast } from 'sonner';
import { Button } from '@/components/ui/button';
import { Callout } from '@/components/ui/callout';
import { DeckIcon } from '@/components/ui/deck-icon';
import { EmptyState, PageLoading } from '@/components/ui/empty-state';
import { IconButton } from '@/components/ui/icon-button';
import { ChevronDown, GripVertical } from '@/components/ui/icons';
import { LinkRow } from '@/components/ui/list-row';
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
import { useActionItemMutations } from '@/hooks/useActionItemMutations';
import {
  affectedByBulk,
  assigneeLabel,
  buildListArgs,
  groupActionItems,
  reorderIds,
  sortActionItems,
  type BulkFilter,
  type HubSort,
  type PersonFilter,
  type StatusFilter,
} from '@/lib/action-items';
import { formatMeetingDate } from '@/lib/format-date';
import { safeListen } from '@/lib/safe-listen';
import type { ActionItemWithMeeting, Person } from '@/types';
import { cn } from '@/lib/utils';
import { PageHeader } from '@/components/ui/page-header';
import { PAGE_CONTENT_WIDTH, PAGE_GUTTER } from '@/lib/page-width';
import { SegmentedControl } from '@/components/ui/segmented-control';

const STATUS_SEGMENTS: Array<{ value: StatusFilter; label: string }> = [
  { value: 'open', label: 'Open' },
  { value: 'completed', label: 'Completed' },
  { value: 'dismissed', label: 'Dismissed' },
];

// WS1.c — whose items. "By person" is its own filter menu beside this switch: it needs a
// person picked, which a segment can't ask for.
const WHOSE_SEGMENTS: Array<{ value: 'mine' | 'everyone'; label: string }> = [
  { value: 'mine', label: 'Mine' },
  { value: 'everyone', label: 'Everyone' },
];

const SORT_OPTIONS: Array<{ value: HubSort; label: string }> = [
  { value: 'meeting', label: 'Meeting date' },
  { value: 'manual', label: 'Manual' },
  { value: 'due', label: 'Due date' },
];

const EMPTY_COPY: Record<StatusFilter, string> = {
  open: 'No open action items. They appear here as meeting summaries are generated — or add one yourself.',
  completed: 'Nothing completed yet.',
  dismissed: 'No dismissed items.',
};

/** Compact deep-link chip to an item's source meeting (WS1.b flat views). */
function MeetingChip({
  title,
  createdAt,
  onOpen,
}: {
  title: string;
  createdAt: string | null;
  onOpen: () => void;
}) {
  const date = formatMeetingDate(createdAt);
  return (
    <Button
      variant="outline"
      size="xs"
      onClick={onOpen}
      title="Open this meeting"
      className="h-auto max-w-[220px] gap-1 px-2 py-px text-caption font-normal text-muted-foreground shadow-none"
    >
      <span className="truncate">{title}</span>
      {date && <span className="flex-shrink-0 opacity-70">· {date}</span>}
    </Button>
  );
}

export default function TasksPage() {
  const router = useRouter();

  const [items, setItems] = useState<ActionItemWithMeeting[]>([]);
  const [people, setPeople] = useState<Person[]>([]);
  const [isLoading, setIsLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [status, setStatus] = useState<StatusFilter>('open');
  // WS1.c: default to "Mine" (owner's primary workflow) — Mine + Open out of the box.
  const [person, setPerson] = useState<PersonFilter>({ kind: 'me' });
  // WS1.b: Meeting-date grouping is the default order.
  const [sort, setSort] = useState<HubSort>('meeting');

  // WS1.b drag state (Manual sort only).
  const [draggingId, setDraggingId] = useState<string | null>(null);
  const [overId, setOverId] = useState<string | null>(null);
  const [armedId, setArmedId] = useState<string | null>(null);
  const resetDrag = useCallback(() => {
    setDraggingId(null);
    setOverId(null);
    setArmedId(null);
  }, []);

  const load = useCallback(async () => {
    try {
      const result = await invoke<ActionItemWithMeeting[]>(
        'api_list_action_items',
        buildListArgs(status, person),
      );
      setItems(Array.isArray(result) ? result : []);
      setError(null);
    } catch (err) {
      console.error('Failed to load action items:', err);
      setError('Could not load your action items.');
    } finally {
      setIsLoading(false);
    }
  }, [status, person]);

  useEffect(() => {
    void load();
  }, [load]);

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

  // Cross-meeting page: refresh on ANY extraction commit.
  useEffect(() => safeListen('action-items-updated', () => void load()), [load]);

  const nameById = useMemo(
    () => new Map(people.map((p) => [p.id, p.displayName])),
    [people],
  );
  const candidates = useMemo<AssigneeCandidate[]>(
    () =>
      people.map((p) => ({ personId: p.id, displayName: p.displayName, email: p.email })),
    [people],
  );
  const groups = useMemo(() => groupActionItems(items), [items]);
  const flatItems = useMemo(
    () => (sort === 'meeting' ? [] : sortActionItems(items, sort)),
    [items, sort],
  );

  const personName =
    person.kind === 'person' ? (nameById.get(person.personId) ?? 'Person') : null;

  // Row mutations (shared hook): optimistic with per-item error reverts.
  const {
    setStatus: setItemStatus,
    editDescription,
    assign,
    deleteItem,
    add,
    setDueDate,
    reorder,
    bulkSetStatus,
  } = useActionItemMutations<ActionItemWithMeeting>({
    setItems,
    meetingId: null, // the hub's add row creates standalone to-dos
    // Bare IPC items lack the meeting join fields; real rows keep their own.
    toRow: (item) => ({ meetingTitle: null, meetingCreatedAt: null, ...item }),
    onStatusChanged: (updated) => {
      // A status change moves the item out of the current status filter —
      // drop it locally instead of re-querying the whole list.
      if (updated.status !== status) {
        setItems((prev) => prev.filter((i) => i.id !== updated.id));
      }
    },
    onAssigned: async () => {
      // The change may move the item out of a person-scoped view.
      if (person.kind !== 'anyone') await load();
    },
    onAdded: () => toast.success('To-do added'),
    // A new item is 'open' and unassigned — keep it visible in whatever person view
    // it was just created from (you added it), as long as the status filter matches.
    appendCreated: (created) => created.status === status,
  });

  // WS1.b — persist a drag drop as a new manual order (optimistic in the hook).
  const handleDrop = useCallback(
    (targetId: string) => {
      if (draggingId && draggingId !== targetId) {
        const next = reorderIds(
          flatItems.map((i) => i.id),
          draggingId,
          targetId,
        );
        void reorder(next);
      }
      resetDrag();
    },
    [draggingId, flatItems, reorder, resetDrag],
  );

  // WS1.e — bulk dismiss, with a client-side id snapshot backing the Undo.
  const handleBulkDismiss = useCallback(
    async (filter: BulkFilter, label: string) => {
      const ids = affectedByBulk(items, filter);
      if (ids.length === 0) {
        toast('Nothing to dismiss', { description: `No open items ${label}.` });
        return;
      }
      const affected = new Set(ids);
      const snapshot = items;
      setItems((prev) => prev.filter((i) => !affected.has(i.id)));
      try {
        const count = await bulkSetStatus(filter, 'dismissed');
        toast.success(`Dismissed ${count} ${count === 1 ? 'item' : 'items'}`, {
          description: 'They won’t come back on the next scan.',
          action: {
            label: 'Undo',
            onClick: () => {
              void (async () => {
                try {
                  await bulkSetStatus({ mode: 'ids', ids }, 'open');
                  await load();
                } catch (err) {
                  toast.error('Could not undo', {
                    description: err instanceof Error ? err.message : String(err),
                  });
                }
              })();
            },
          },
        });
      } catch (err) {
        setItems(() => snapshot);
        toast.error('Could not dismiss those items', {
          description: err instanceof Error ? err.message : String(err),
        });
      }
    },
    [items, bulkSetStatus, load],
  );

  const canDismiss = status === 'open';
  const notMeCount = useMemo(
    () => (canDismiss ? affectedByBulk(items, { mode: 'notSelf' }).length : 0),
    [items, canDismiss],
  );

  const viewSegment: 'mine' | 'everyone' | 'byPerson' =
    person.kind === 'me' ? 'mine' : person.kind === 'anyone' ? 'everyone' : 'byPerson';

  const rowProps = (item: ActionItemWithMeeting) =>
    ({
      item,
      assigneeName: assigneeLabel(item, nameById),
      candidates,
      onSetStatus: setItemStatus,
      onEditDescription: editDescription,
      onAssign: assign,
      onDelete: deleteItem,
    }) as const;

  return (
    // A short opacity fade, no travel (docs/design-system.md §Motion).
    <div className="flex h-page animate-fade-in flex-col bg-background">
      <PageHeader
        width="page"
        title="Action items"
        subtitle={
          items.length > 0
            ? `${items.length} ${items.length === 1 ? 'item' : 'items'}`
            : 'Commitments from your meetings, in one place'
        }
      />

      {/* Filter bar: status · view (Mine/Everyone/By person) · sort · bulk. */}
      <div className={cn('flex-shrink-0 pb-3', PAGE_GUTTER)}>
        <div className={cn('mx-auto flex w-full flex-wrap items-center gap-2', PAGE_CONTENT_WIDTH)}>
          <SegmentedControl
            aria-label="Filter by status"
            options={STATUS_SEGMENTS}
            value={status}
            onChange={(value) => setStatus(value as StatusFilter)}
          />

          {/* WS1.c — top-level view: Mine / Everyone, the same SegmentedControl as status
              (one segmented style on the page). A person filter leaves neither pressed. A
              hairline divides the two switches, or they read as one five-way control. */}
          <span aria-hidden="true" className="mx-1 h-5 w-px bg-border" />
          <SegmentedControl
            aria-label="Whose items"
            options={WHOSE_SEGMENTS}
            value={viewSegment}
            onChange={(value) => setPerson(value === 'everyone' ? { kind: 'anyone' } : { kind: 'me' })}
          />

          {/* "By person" — a filter menu; its label is the person once one is picked. */}
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <Button
                variant="outline"
                size="sm"
                aria-label="Filter by a specific person"
                data-active={viewSegment === 'byPerson' || undefined}
                className={cn('gap-1.5', viewSegment === 'byPerson' && 'border-brand-edge')}
              >
                {viewSegment === 'byPerson' ? personName : 'By person'}
                <DeckIcon icon={ChevronDown} size={12} className="text-muted-foreground" />
              </Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="start">
              {people.length === 0 ? (
                <DropdownMenuItem disabled>No people yet</DropdownMenuItem>
              ) : (
                people.map((p) => (
                  <DropdownMenuItem
                    key={p.id}
                    onSelect={() => setPerson({ kind: 'person', personId: p.id })}
                  >
                    {p.displayName}
                  </DropdownMenuItem>
                ))
              )}
            </DropdownMenuContent>
          </DropdownMenu>

          {/* WS1.b — sort control. */}
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <Button variant="outline" size="sm" aria-label="Sort" className="gap-1.5">
                {SORT_OPTIONS.find((o) => o.value === sort)?.label}
                <DeckIcon icon={ChevronDown} size={12} className="text-muted-foreground" />
              </Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="start">
              {SORT_OPTIONS.map((o) => (
                <DropdownMenuItem key={o.value} onSelect={() => setSort(o.value)}>
                  {o.label}
                </DropdownMenuItem>
              ))}
            </DropdownMenuContent>
          </DropdownMenu>

          {/* WS1.e — bulk dismiss (Open view only; matches the backend's open-only rule). */}
          {canDismiss && (
            <DropdownMenu>
              <DropdownMenuTrigger asChild>
                <Button variant="outline" size="sm" aria-label="Bulk dismiss" className="ml-auto gap-1.5">
                  Dismiss
                  <DeckIcon icon={ChevronDown} size={12} className="text-muted-foreground" />
                </Button>
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end">
                <DropdownMenuItem
                  disabled={notMeCount === 0}
                  onSelect={() =>
                    void handleBulkDismiss({ mode: 'notSelf' }, 'assigned to other people')
                  }
                >
                  Dismiss all not assigned to me
                  {notMeCount > 0 && (
                    <span className="ml-auto pl-3 text-xs text-muted-foreground">
                      {notMeCount}
                    </span>
                  )}
                </DropdownMenuItem>
                {person.kind === 'person' && personName && (
                  <>
                    <DropdownMenuSeparator />
                    <DropdownMenuItem
                      onSelect={() =>
                        void handleBulkDismiss(
                          { mode: 'person', personId: person.personId },
                          `from ${personName}`,
                        )
                      }
                    >
                      Dismiss all from {personName}
                    </DropdownMenuItem>
                  </>
                )}
              </DropdownMenuContent>
            </DropdownMenu>
          )}
        </div>
      </div>

      <div className={cn('flex-1 overflow-y-auto pb-12', PAGE_GUTTER)}>
        <div className={cn('mx-auto w-full', PAGE_CONTENT_WIDTH)}>
          {/* Standalone to-do entry — creates an "Unattached" manual item. */}
          <AddActionItemRow placeholder="New item — add a to-do…" onAdd={add} />

          {isLoading ? (
            <PageLoading fill="pane" label="Loading action items…" />
          ) : error ? (
            <Callout tone="error" className="mt-4">
              {error}
            </Callout>
          ) : items.length === 0 ? (
            <EmptyState
              className="h-[40vh]"
              title={status === 'open' ? 'No action items' : 'Nothing here'}
              body={EMPTY_COPY[status]}
            />
          ) : sort === 'meeting' ? (
            /* Grouped-by-meeting (default). */
            <div className="mt-4 flex flex-col gap-5">
              {groups.map((group) => (
                <section key={group.key} aria-label={group.title}>
                  {group.meetingId ? (
                    <LinkRow
                      title={group.title}
                      meta={formatMeetingDate(group.createdAt) || undefined}
                      onClick={() => router.push(`/meeting-details?id=${group.meetingId}`)}
                      hint="Open this meeting"
                    />
                  ) : (
                    <div className="u-row-title px-2 py-1.5">{group.title}</div>
                  )}
                  <div className="mt-0.5 flex flex-col gap-0.5">
                    {group.items.map((item) => (
                      <ActionItemRow key={item.id} {...rowProps(item)} onSetDueDate={setDueDate} />
                    ))}
                  </div>
                </section>
              ))}
            </div>
          ) : (
            /* Flat list (Manual / Due) — meeting becomes a row chip; Manual is drag-orderable. */
            <div className="mt-4 flex flex-col gap-0.5">
              {flatItems.map((item) => (
                <div
                  key={item.id}
                  draggable={sort === 'manual' && armedId === item.id}
                  onDragStart={(e) => {
                    setDraggingId(item.id);
                    if (e.dataTransfer) e.dataTransfer.effectAllowed = 'move';
                  }}
                  onDragOver={(e) => {
                    if (sort === 'manual' && draggingId) {
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
                    {...rowProps(item)}
                    compact
                    onSetDueDate={setDueDate}
                    meetingChip={
                      item.meetingId ? (
                        <MeetingChip
                          title={item.meetingTitle?.trim() || 'Untitled meeting'}
                          createdAt={item.meetingCreatedAt}
                          onOpen={() =>
                            router.push(`/meeting-details?id=${item.meetingId}`)
                          }
                        />
                      ) : null
                    }
                    dragHandle={
                      sort === 'manual' ? (
                        <IconButton
                          variant="ghost"
                          label="Drag to reorder"
                          onMouseDown={() => setArmedId(item.id)}
                          onMouseUp={() => setArmedId(null)}
                          icon={<DeckIcon icon={GripVertical} size={14} />}
                          // A narrow grip in the row's gutter, not a 28px key: it shows on
                          // row hover and reads as a handle, not a button.
                          className="mt-0.5 h-[18px] w-4 cursor-grab text-muted-foreground/40 opacity-0 transition-opacity hover:bg-transparent hover:text-muted-foreground focus-visible:opacity-100 group-hover:opacity-100 active:cursor-grabbing"
                        />
                      ) : null
                    }
                  />
                </div>
              ))}
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
