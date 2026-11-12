import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { render, screen, fireEvent, waitFor, within } from '@testing-library/react';
import TasksPage from '@/app/tasks/page';
import type { Person, TodoRow, TodoView } from '@/types';

// specs/0088 — the To-do / Inbox / Done page. Locks: (a) three tabs with the inbox count
// and the ?tab= sync, (b) Add to my list moves a suggestion from Inbox to the To-do list,
// (c) quick-add splits a trailing due phrase, (d) dismiss-older-than shows an Undo toast
// carrying the count, (e) Done groups by Monday-start week, (f) the To-do tab is one flat
// list with no meeting grouping, (g) Show dismissed + Restore, (h) a failed load shows a
// callout.

const pushMock = vi.fn();
const replaceMock = vi.fn();
let searchParams = new URLSearchParams();
vi.mock('next/navigation', () => ({
  useRouter: () => ({ push: pushMock, replace: replaceMock }),
  useSearchParams: () => searchParams,
}));
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn() }));
vi.mock('sonner', () => {
  const base = vi.fn();
  return { toast: Object.assign(base, { success: vi.fn(), error: vi.fn() }) };
});

import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { toast } from 'sonner';

const invokeMock = vi.mocked(invoke);
const listenMock = vi.mocked(listen);
const toastSuccess = vi.mocked(toast.success);

const PEOPLE: Person[] = [
  {
    id: 'p1',
    displayName: 'Alice Chen',
    email: 'alice@acme.io',
    role: null,
    notes: null,
    voiceprintOptOut: false,
    starred: false,
    createdAt: '',
    updatedAt: '',
  },
];

function todoRow(overrides: Partial<TodoRow>): TodoRow {
  return {
    id: 'ai-1',
    meetingId: 'm1',
    description: 'Send the deck',
    assigneePersonId: null,
    assigneeIsSelf: false,
    assigneeRaw: null,
    dueHint: null,
    dueDate: null,
    status: 'open',
    source: 'extracted',
    userEdited: false,
    contentKey: 'ck',
    createdAt: '2026-11-01T10:00:00Z',
    updatedAt: '2026-11-01T10:00:00Z',
    completedAt: null,
    sortOrder: null,
    triage: 'listed',
    adoptedAt: null,
    notesMarkdown: null,
    meetingTitle: 'Weekly sync',
    meetingCreatedAt: '2026-11-01T09:00:00Z',
    sourceCount: 1,
    latestSourceAt: '2026-11-01T09:00:00Z',
    latestContext: null,
    ...overrides,
  };
}

interface Store {
  rows: TodoRow[];
}

const inView = (r: TodoRow, view: TodoView): boolean => {
  if (view === 'list') return r.triage === 'listed' && r.status === 'open';
  if (view === 'inbox') return r.triage === 'suggested' && r.status === 'open';
  if (view === 'done') return r.status === 'completed';
  return r.status === 'dismissed';
};

function stubInvoke(
  store: Store,
  handlers: Record<string, (args?: any) => unknown> = {}, // eslint-disable-line @typescript-eslint/no-explicit-any
) {
  invokeMock.mockImplementation((async (cmd: string, args?: any) => { // eslint-disable-line @typescript-eslint/no-explicit-any
    if (cmd in handlers) return handlers[cmd]!(args);
    if (cmd === 'api_list_todo_view') return store.rows.filter((r) => inView(r, args.view));
    if (cmd === 'api_inbox_count') return store.rows.filter((r) => inView(r, 'inbox')).length;
    if (cmd === 'api_list_people') return PEOPLE;
    if (cmd === 'api_triage_action_item') {
      const r = store.rows.find((x) => x.id === args.id)!;
      if (args.action === 'add_to_list') r.triage = 'listed';
      if (args.action === 'dismiss') r.status = 'dismissed';
      if (args.action === 'restore') r.status = 'open';
      return r;
    }
    if (cmd === 'api_create_action_item') {
      const created = todoRow({
        id: 'ai-new',
        meetingId: null,
        meetingTitle: null,
        meetingCreatedAt: null,
        source: 'manual',
        description: args.description,
        dueDate: args.dueDate ?? null,
      });
      store.rows.push(created);
      return created;
    }
    if (cmd === 'api_set_action_item_status') {
      const r = store.rows.find((x) => x.id === args.id)!;
      r.status = args.status;
      return r;
    }
    if (cmd === 'api_reorder_action_items') return 0;
    if (cmd === 'api_bulk_set_action_item_status') return 1;
    throw new Error(`Unexpected command in test: ${cmd}`);
  }) as unknown as typeof invoke);
}

const callsTo = (cmd: string) => invokeMock.mock.calls.filter(([c]) => c === cmd);

/** Radix tabs activate on mousedown (not click). */
function pickTab(name: string) {
  fireEvent.mouseDown(screen.getByRole('tab', { name: new RegExp(`^${name}`) }), { button: 0 });
}

/** Open a Radix dropdown by its trigger aria-label (keyboard, jsdom-friendly). */
function openMenu(label: string) {
  const trigger = screen.getByRole('button', { name: label });
  trigger.focus();
  fireEvent.keyDown(trigger, { key: 'Enter' });
}

beforeEach(() => {
  vi.clearAllMocks();
  searchParams = new URLSearchParams();
  listenMock.mockResolvedValue(() => {});
});

afterEach(() => {
  vi.useRealTimers();
});

const inboxRow = (id: string, description: string, over: Partial<TodoRow> = {}) =>
  todoRow({ id, description, triage: 'suggested', ...over });

describe('TasksPage (specs/0088 To-do / Inbox / Done)', () => {
  it('renders three tabs with inbox count', async () => {
    stubInvoke({ rows: [inboxRow('a', 'Draft the plan'), inboxRow('b', 'Book the room')] });
    render(<TasksPage />);

    const todo = await screen.findByRole('tab', { name: /^To-do/ });
    const inbox = screen.getByRole('tab', { name: /^Inbox/ });
    const done = screen.getByRole('tab', { name: /^Done/ });
    expect([todo, inbox, done]).toHaveLength(3);
    await waitFor(() => expect(within(inbox).getByText('2')).toBeInTheDocument());
    expect(todo).toHaveAttribute('data-state', 'active');
  });

  it('syncs the selected tab to ?tab=', async () => {
    searchParams = new URLSearchParams('tab=done');
    stubInvoke({ rows: [] });
    render(<TasksPage />);
    expect(await screen.findByRole('tab', { name: /^Done/ })).toHaveAttribute('data-state', 'active');

    pickTab('Inbox');
    expect(replaceMock).toHaveBeenCalledWith('/tasks?tab=inbox');
  });

  it('add to my list moves row from inbox to to-do', async () => {
    searchParams = new URLSearchParams('tab=inbox');
    stubInvoke({ rows: [inboxRow('a', 'Draft the plan')] });
    render(<TasksPage />);

    fireEvent.click(await screen.findByRole('button', { name: 'Add to my list' }));
    await waitFor(() =>
      expect(callsTo('api_triage_action_item')[0]![1]).toEqual({ id: 'a', action: 'add_to_list' }),
    );
    expect(await screen.findByText('Nothing waiting')).toBeInTheDocument();

    pickTab('To-do');
    expect(await screen.findByText('Draft the plan')).toBeInTheDocument();
  });

  it('quick add parses due date', async () => {
    vi.useFakeTimers({ toFake: ['Date'], now: new Date(2026, 10, 12, 9, 0, 0) });
    stubInvoke({ rows: [] });
    render(<TasksPage />);

    const input = await screen.findByLabelText('New item — add a to-do…');
    fireEvent.change(input, { target: { value: 'Send the deck (Nov 20)' } });
    fireEvent.submit(input);

    await waitFor(() =>
      expect(callsTo('api_create_action_item')[0]![1]).toEqual({
        meetingId: null,
        description: 'Send the deck',
        dueDate: '2026-11-20',
      }),
    );
  });

  it('dismiss older than shows undo with count', async () => {
    searchParams = new URLSearchParams('tab=inbox');
    stubInvoke(
      { rows: [inboxRow('a', 'Old one'), inboxRow('b', 'Older one')] },
      { api_dismiss_suggestions_older_than: () => ['a', 'b'] },
    );
    render(<TasksPage />);
    await screen.findByText('Old one');

    openMenu('Inbox options');
    fireEvent.click(await screen.findByText('Dismiss suggestions older than 30 days'));

    await waitFor(() =>
      expect(callsTo('api_dismiss_suggestions_older_than')[0]![1]).toEqual({ days: 30 }),
    );
    await waitFor(() => expect(toastSuccess).toHaveBeenCalled());
    expect(toastSuccess.mock.calls.at(-1)![0]).toBe('Dismissed 2 suggestions');
    const opts = toastSuccess.mock.calls.at(-1)![1] as unknown as { action: { onClick: () => void } };
    opts.action.onClick();
    await waitFor(() =>
      expect(callsTo('api_bulk_set_action_item_status').at(-1)![1]).toEqual({
        filter: { mode: 'ids', ids: ['a', 'b'] },
        status: 'open',
      }),
    );
  });

  it('done groups by week', async () => {
    // Thursday 12 Nov 2026 -> this week starts Mon 9 Nov, last week Mon 2 Nov.
    vi.useFakeTimers({ toFake: ['Date'], now: new Date(2026, 10, 12, 9, 0, 0) });
    searchParams = new URLSearchParams('tab=done');
    const done = (id: string, description: string, completedAt: string) =>
      todoRow({ id, description, status: 'completed', completedAt });
    stubInvoke({
      rows: [
        done('d1', 'Finished today', new Date(2026, 10, 11, 12).toISOString()),
        done('d2', 'Finished last week', new Date(2026, 10, 3, 12).toISOString()),
        done('d3', 'Finished long ago', new Date(2026, 9, 21, 12).toISOString()),
      ],
    });
    render(<TasksPage />);

    expect(await screen.findByText('This week')).toBeInTheDocument();
    expect(screen.getByText('Last week')).toBeInTheDocument();
    expect(screen.getByText('Week of Oct 19')).toBeInTheDocument();
    expect(
      screen.getByRole('checkbox', { name: 'Mark "Finished today" as open' }),
    ).toBeInTheDocument();
  });

  it('no meeting grouping on to-do tab', async () => {
    stubInvoke({
      rows: [
        todoRow({ id: 'a', description: 'Send the deck' }),
        todoRow({ id: 'b', description: 'Renew the domain', meetingId: null, meetingTitle: null, source: 'manual' }),
      ],
    });
    render(<TasksPage />);

    expect(await screen.findByText('Send the deck')).toBeInTheDocument();
    expect(screen.getByText('Renew the domain')).toBeInTheDocument();
    expect(screen.queryByText('Weekly sync')).not.toBeInTheDocument();
    expect(screen.queryByText('Unattached')).not.toBeInTheDocument();
    // No sort switcher, no person filter.
    expect(screen.queryByRole('button', { name: 'Sort' })).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Filter by a specific person' })).not.toBeInTheDocument();
  });

  it('a drag drop reorders the rows on screen', async () => {
    stubInvoke({
      rows: [
        todoRow({ id: 'a', description: 'First item', sortOrder: 0 }),
        todoRow({ id: 'b', description: 'Second item', sortOrder: 1 }),
      ],
    });
    render(<TasksPage />);
    await screen.findByText('First item');
    const order = () =>
      screen.getAllByRole('button', { name: /^(First|Second) item$/ }).map((b) => b.textContent);
    expect(order()).toEqual(['First item', 'Second item']);

    fireEvent.dragStart(screen.getByText('Second item'));
    fireEvent.dragOver(screen.getByText('First item'));
    fireEvent.drop(screen.getByText('First item'));

    await waitFor(() => expect(order()).toEqual(['Second item', 'First item']));
    expect(callsTo('api_reorder_action_items').at(-1)![1]).toEqual({ orderedIds: ['b', 'a'] });
  });

  it('opens an item from its title', async () => {
    stubInvoke({ rows: [todoRow({ id: 'a', description: 'Send the deck' })] });
    render(<TasksPage />);
    fireEvent.click(await screen.findByRole('button', { name: 'Send the deck' }));
    expect(pushMock).toHaveBeenCalledWith('/tasks/item?id=a');
  });

  it('assigning an inbox suggestion to someone else updates it and the row leaves', async () => {
    searchParams = new URLSearchParams('tab=inbox');
    const store = { rows: [inboxRow('a', 'Draft the plan')] };
    stubInvoke(store, {
      api_update_action_item: () => {
        // The backend moves a reassigned suggestion out of the inbox view.
        store.rows[0]!.assigneePersonId = 'p1';
        store.rows[0]!.triage = 'record';
        return store.rows[0];
      },
    });
    render(<TasksPage />);

    fireEvent.click(await screen.findByRole('button', { name: 'Assign…' }));
    fireEvent.click(await screen.findByText('Alice Chen'));
    await waitFor(() =>
      expect(callsTo('api_update_action_item')[0]![1]).toEqual({ id: 'a', assigneePersonId: 'p1' }),
    );
    await waitFor(() => expect(screen.queryByText('Draft the plan')).not.toBeInTheDocument());
    expect(await screen.findByText('Nothing waiting')).toBeInTheDocument();
  });

  it('Show dismissed swaps in the dismissed list with Restore', async () => {
    searchParams = new URLSearchParams('tab=inbox');
    stubInvoke({
      rows: [
        inboxRow('a', 'Live suggestion'),
        inboxRow('x', 'Dismissed suggestion', { status: 'dismissed' }),
      ],
    });
    render(<TasksPage />);
    await screen.findByText('Live suggestion');

    openMenu('Inbox options');
    fireEvent.click(await screen.findByText('Show dismissed'));
    expect(await screen.findByText('Dismissed suggestion')).toBeInTheDocument();
    expect(screen.queryByText('Live suggestion')).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: 'Restore' }));
    await waitFor(() =>
      expect(callsTo('api_triage_action_item').at(-1)![1]).toEqual({ id: 'x', action: 'restore' }),
    );
  });

  it('shows a callout when the lists cannot load', async () => {
    const errSpy = vi.spyOn(console, 'error').mockImplementation(() => {});
    stubInvoke({ rows: [] }, { api_list_todo_view: () => { throw new Error('boom'); } });
    render(<TasksPage />);
    expect(await screen.findByText('Could not load your to-do list.')).toBeInTheDocument();
    errSpy.mockRestore();
  });
});
