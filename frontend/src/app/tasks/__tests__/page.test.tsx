import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import TasksPage from '@/app/tasks/page';
import type { ActionItemWithMeeting, Person } from '@/types';

// specs/0034 + specs/0038 WS1 — the cross-meeting task hub. Locks: (a) the default
// query is Mine + Open (WS1.c), (b) the status segments and the Mine/Everyone/By-person
// view control re-query with the right args, (c) items group under meeting headers in
// the default Meeting sort and switch to a flat list in Manual/Due, (d) the new-item row
// creates a standalone manual to-do and appends it locally, (e) a status change drops the
// item from the current filter locally, (f) Manual drag persists via
// api_reorder_action_items, (g) bulk "not me" dismiss dispatches the filter and the Undo
// re-opens exactly the snapshotted ids.

const pushMock = vi.fn();
vi.mock('next/navigation', () => ({
  useRouter: () => ({ push: pushMock }),
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

function hubItem(overrides: Partial<ActionItemWithMeeting>): ActionItemWithMeeting {
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
    createdAt: '2026-07-01T10:00:00Z',
    updatedAt: '2026-07-01T10:00:00Z',
    completedAt: null,
    sortOrder: null,
    meetingTitle: 'Weekly sync',
    meetingCreatedAt: '2026-07-01T09:00:00Z',
    ...overrides,
  };
}

const ITEMS: ActionItemWithMeeting[] = [
  hubItem({ id: 'ai-1' }),
  hubItem({
    id: 'ai-2',
    meetingId: null,
    meetingTitle: null,
    meetingCreatedAt: null,
    description: 'Renew the domain',
    source: 'manual',
  }),
];

function stubInvoke(
  items: ActionItemWithMeeting[] = ITEMS,
  handlers: Record<string, (args?: unknown) => unknown> = {},
) {
  invokeMock.mockImplementation(async (cmd: string, args?: unknown) => {
    if (cmd in handlers) return handlers[cmd]!(args);
    if (cmd === 'api_list_action_items') return items;
    if (cmd === 'api_list_people') return PEOPLE;
    if (cmd === 'api_reorder_action_items') return 0;
    if (cmd === 'api_bulk_set_action_item_status') return 1;
    if (cmd === 'api_create_action_item') {
      const { description } = args as { description: string };
      const {
        meetingTitle: _title,
        meetingCreatedAt: _created,
        ...bare
      } = hubItem({ id: 'ai-new', meetingId: null, source: 'manual', description });
      return bare;
    }
    throw new Error(`Unexpected command in test: ${cmd}`);
  });
}

function listCalls() {
  return invokeMock.mock.calls.filter(([cmd]) => cmd === 'api_list_action_items');
}
function callsTo(cmd: string) {
  return invokeMock.mock.calls.filter(([c]) => c === cmd);
}

/** Open a Radix dropdown by its trigger aria-label (keyboard, jsdom-friendly). */
function openMenu(label: string) {
  const trigger = screen.getByRole('button', { name: label });
  trigger.focus();
  fireEvent.keyDown(trigger, { key: 'Enter' });
}

beforeEach(() => {
  vi.clearAllMocks();
  listenMock.mockResolvedValue(() => {});
});

describe('TasksPage (specs/0034 + 0038 WS1 hub)', () => {
  it('defaults to Mine + Open and groups items by meeting (WS1.c)', async () => {
    stubInvoke();
    render(<TasksPage />);

    expect(await screen.findByText('Weekly sync')).toBeInTheDocument();
    expect(screen.getByText('Send the deck')).toBeInTheDocument();
    expect(screen.getByText('Unattached')).toBeInTheDocument();

    expect(listCalls()[0]![1]).toEqual({
      status: 'open',
      personId: null,
      mineOnly: true,
    });
  });

  it('deep-links a meeting header to /meeting-details?id=…', async () => {
    stubInvoke();
    render(<TasksPage />);

    fireEvent.click(await screen.findByRole('button', { name: /Weekly sync/ }));
    expect(pushMock).toHaveBeenCalledWith('/meeting-details?id=m1');
  });

  it('re-queries when the status segment changes', async () => {
    stubInvoke();
    render(<TasksPage />);
    await screen.findByText('Weekly sync');

    fireEvent.click(screen.getByRole('button', { name: 'Completed' }));

    await waitFor(() =>
      expect(listCalls().at(-1)![1]).toEqual({
        status: 'completed',
        personId: null,
        mineOnly: true,
      }),
    );
  });

  it('view control: Everyone drops mineOnly, By-person queries by id (WS1.c)', async () => {
    stubInvoke();
    render(<TasksPage />);
    await screen.findByText('Weekly sync');

    fireEvent.click(screen.getByRole('button', { name: 'Everyone' }));
    await waitFor(() =>
      expect(listCalls().at(-1)![1]).toEqual({
        status: 'open',
        personId: null,
        mineOnly: false,
      }),
    );

    openMenu('Filter by a specific person');
    fireEvent.click(await screen.findByText('Alice Chen'));
    await waitFor(() =>
      expect(listCalls().at(-1)![1]).toEqual({
        status: 'open',
        personId: 'p1',
        mineOnly: false,
      }),
    );
  });

  it('completes an item: drops it from the Open view locally, without a re-query', async () => {
    stubInvoke(ITEMS, {
      api_set_action_item_status: () => ({
        ...hubItem({ id: 'ai-1' }),
        status: 'completed',
      }),
    });
    render(<TasksPage />);
    await screen.findByText('Send the deck');
    const queriesBefore = listCalls().length;

    fireEvent.click(
      screen.getByRole('checkbox', { name: 'Mark "Send the deck" as completed' }),
    );

    await waitFor(() =>
      expect(screen.queryByText('Send the deck')).not.toBeInTheDocument(),
    );
    expect(screen.getByText('Renew the domain')).toBeInTheDocument();
    expect(listCalls().length).toBe(queriesBefore);
  });

  it('creates a standalone manual to-do and appends it locally (no re-query)', async () => {
    stubInvoke();
    render(<TasksPage />);
    await screen.findByText('Weekly sync');
    const queriesBefore = listCalls().length;

    const input = screen.getByLabelText('New item — add a to-do…');
    fireEvent.change(input, { target: { value: 'File the expense report' } });
    fireEvent.submit(input);

    await waitFor(() => {
      const call = callsTo('api_create_action_item')[0];
      expect(call?.[1]).toEqual({
        meetingId: null,
        description: 'File the expense report',
      });
    });
    expect(await screen.findByText('File the expense report')).toBeInTheDocument();
    expect(listCalls().length).toBe(queriesBefore);
    expect(input).toHaveValue('');
  });

  it('Manual sort switches to a flat list and a drag persists the new order (WS1.b)', async () => {
    stubInvoke();
    render(<TasksPage />);
    await screen.findByText('Weekly sync');

    // Switch Meeting → Manual: the flat list exposes per-row drag handles.
    openMenu('Sort');
    fireEvent.click(await screen.findByText('Manual'));
    await waitFor(() =>
      expect(screen.getAllByRole('button', { name: 'Drag to reorder' })).toHaveLength(2),
    );

    // Drag "Renew the domain" (ai-2) onto "Send the deck" (ai-1) → order [ai-2, ai-1].
    const dragged = screen.getByText('Renew the domain');
    const target = screen.getByText('Send the deck');
    fireEvent.dragStart(dragged);
    fireEvent.dragOver(target);
    fireEvent.drop(target);

    await waitFor(() =>
      expect(callsTo('api_reorder_action_items').at(-1)![1]).toEqual({
        orderedIds: ['ai-2', 'ai-1'],
      }),
    );
  });

  it('bulk-dismisses items not assigned to me, and Undo re-opens exactly those ids (WS1.e)', async () => {
    const items = [
      hubItem({ id: 'ai-self', description: 'My task', assigneeIsSelf: true }),
      hubItem({ id: 'ai-other', description: 'Their task', assigneePersonId: 'p1' }),
    ];
    stubInvoke(items);
    render(<TasksPage />);
    await screen.findByText('Their task');

    openMenu('Bulk dismiss');
    fireEvent.click(await screen.findByText('Dismiss all not assigned to me'));

    // The filter dispatched, and the other-person item leaves the Open view.
    await waitFor(() =>
      expect(callsTo('api_bulk_set_action_item_status')[0]![1]).toEqual({
        filter: { mode: 'notSelf' },
        status: 'dismissed',
      }),
    );
    await waitFor(() =>
      expect(screen.queryByText('Their task')).not.toBeInTheDocument(),
    );
    expect(screen.getByText('My task')).toBeInTheDocument();

    // Undo re-opens exactly the snapshotted ids via the ids-mode filter.
    const undo = toastSuccess.mock.calls.at(-1)![1] as unknown as {
      action: { onClick: () => void };
    };
    undo.action.onClick();
    await waitFor(() =>
      expect(callsTo('api_bulk_set_action_item_status').at(-1)![1]).toEqual({
        filter: { mode: 'ids', ids: ['ai-other'] },
        status: 'open',
      }),
    );
  });

  it('shows the per-status empty state', async () => {
    stubInvoke([]);
    render(<TasksPage />);

    expect(await screen.findByText('No action items')).toBeInTheDocument();
  });
});
