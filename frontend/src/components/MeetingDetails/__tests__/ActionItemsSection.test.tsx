import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import { ActionItemsSection } from '@/components/MeetingDetails/ActionItemsSection';
import type { ActionItem } from '@/types';

// specs/0034 — the per-meeting Action items section. Locks the core interactions:
// complete/edit/dismiss/delete/add/scan call the right IPC commands with the right
// args, the two empty states render per the spec, the section refreshes on the
// `action-items-updated` Tauri event, a failed mutation reverts ONLY the mutated
// item (never clobbering a concurrent reload), a failed add keeps the typed draft,
// and the scan toast reflects the inserted-or-updated count contract.

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn() }));
vi.mock('sonner', () => ({ toast: { success: vi.fn(), error: vi.fn() } }));

import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { toast } from 'sonner';

const invokeMock = vi.mocked(invoke);
const listenMock = vi.mocked(listen);
const toastSuccess = vi.mocked(toast.success);
const toastError = vi.mocked(toast.error);

function makeItem(overrides: Partial<ActionItem> = {}): ActionItem {
  return {
    id: 'ai-1',
    meetingId: 'm1',
    description: 'Send the deck to Alice',
    assigneePersonId: null,
    assigneeIsSelf: false,
    assigneeRaw: null,
    dueHint: 'by Friday',
    dueDate: null,
    status: 'open',
    source: 'extracted',
    userEdited: false,
    contentKey: 'ck1',
    createdAt: '2026-07-01T10:00:00Z',
    updatedAt: '2026-07-01T10:00:00Z',
    completedAt: null,
    sortOrder: null,
    ...overrides,
  };
}

/** Route the invoke mock by command; unrouted commands resolve to a benign default. */
function stubInvoke(handlers: Record<string, (args?: unknown) => unknown>) {
  invokeMock.mockImplementation(async (cmd: string, args?: unknown) => {
    if (cmd in handlers) return handlers[cmd]!(args);
    if (cmd === 'api_get_meeting_participants' || cmd === 'api_list_people') return [];
    throw new Error(`Unexpected command in test: ${cmd}`);
  });
}

/** Calls the invoke mock received for a given command. */
function callsFor(cmd: string) {
  return invokeMock.mock.calls.filter(([c]) => c === cmd);
}

type UpdatedHandler = (event: { payload: { meeting_id: string } }) => void;

/** Capture the `action-items-updated` handler so tests can fire the event. */
function captureUpdatedHandler() {
  const captured: { handler: UpdatedHandler | null } = { handler: null };
  listenMock.mockImplementation(async (_event, cb) => {
    captured.handler = cb as unknown as UpdatedHandler;
    return () => {};
  });
  return captured;
}

beforeEach(() => {
  vi.clearAllMocks();
  listenMock.mockResolvedValue(() => {});
});

describe('ActionItemsSection (specs/0034)', () => {
  it('loads and renders the meeting items', async () => {
    stubInvoke({ api_get_action_items: () => [makeItem()] });
    render(<ActionItemsSection meetingId="m1" hasSummary />);

    expect(await screen.findByText('Send the deck to Alice')).toBeInTheDocument();
    expect(screen.getByText('by Friday')).toBeInTheDocument();
    expect(callsFor('api_get_action_items')[0]![1]).toEqual({ meetingId: 'm1' });
  });

  it('completes an item via the checkbox → api_set_action_item_status', async () => {
    const item = makeItem();
    stubInvoke({
      api_get_action_items: () => [item],
      api_set_action_item_status: () => ({ ...item, status: 'completed' }),
    });
    render(<ActionItemsSection meetingId="m1" hasSummary />);

    fireEvent.click(await screen.findByRole('checkbox'));

    await waitFor(() =>
      expect(callsFor('api_set_action_item_status')[0]![1]).toEqual({
        id: 'ai-1',
        status: 'completed',
      }),
    );
  });

  it('edits the description inline → api_update_action_item', async () => {
    const item = makeItem();
    stubInvoke({
      api_get_action_items: () => [item],
      api_update_action_item: () => ({ ...item, description: 'Send the final deck' }),
    });
    render(<ActionItemsSection meetingId="m1" hasSummary />);

    fireEvent.click(await screen.findByText('Send the deck to Alice'));
    const input = screen.getByLabelText('Edit action item description');
    fireEvent.change(input, { target: { value: 'Send the final deck' } });
    fireEvent.keyDown(input, { key: 'Enter' });

    await waitFor(() =>
      expect(callsFor('api_update_action_item')[0]![1]).toEqual({
        id: 'ai-1',
        description: 'Send the final deck',
      }),
    );
  });

  it('dismisses an extracted item from the overflow menu and hides it', async () => {
    const item = makeItem();
    stubInvoke({
      api_get_action_items: () => [item],
      api_set_action_item_status: () => ({ ...item, status: 'dismissed' }),
    });
    render(<ActionItemsSection meetingId="m1" hasSummary />);

    const trigger = await screen.findByRole('button', { name: /options for/i });
    trigger.focus();
    fireEvent.keyDown(trigger, { key: 'Enter' });
    fireEvent.click(await screen.findByText('Dismiss'));

    await waitFor(() =>
      expect(callsFor('api_set_action_item_status')[0]![1]).toEqual({
        id: 'ai-1',
        status: 'dismissed',
      }),
    );
    // Dismissed items are hidden on the meeting page.
    await waitFor(() =>
      expect(screen.queryByText('Send the deck to Alice')).not.toBeInTheDocument(),
    );
    expect(toastSuccess).toHaveBeenCalledWith(
      'Item dismissed',
      expect.objectContaining({ description: expect.any(String) }),
    );
  });

  it('offers Delete (not Dismiss) for manual items → api_delete_action_item', async () => {
    stubInvoke({
      api_get_action_items: () => [makeItem({ id: 'ai-m', source: 'manual' })],
      api_delete_action_item: () => true,
    });
    render(<ActionItemsSection meetingId="m1" hasSummary />);

    const trigger = await screen.findByRole('button', { name: /options for/i });
    trigger.focus();
    fireEvent.keyDown(trigger, { key: 'Enter' });
    expect(screen.queryByText('Dismiss')).not.toBeInTheDocument();
    fireEvent.click(await screen.findByText('Delete'));

    await waitFor(() =>
      expect(callsFor('api_delete_action_item')[0]![1]).toEqual({ id: 'ai-m' }),
    );
  });

  it('adds a manual item on this meeting → api_create_action_item, clearing the draft', async () => {
    stubInvoke({
      api_get_action_items: () => [makeItem()],
      api_create_action_item: () =>
        makeItem({ id: 'ai-new', description: 'Book the room', source: 'manual' }),
    });
    render(<ActionItemsSection meetingId="m1" hasSummary />);
    await screen.findByText('Send the deck to Alice');

    const input = screen.getByLabelText('Add an action item…');
    fireEvent.change(input, { target: { value: 'Book the room' } });
    fireEvent.submit(input);

    await waitFor(() =>
      expect(callsFor('api_create_action_item')[0]![1]).toEqual({
        meetingId: 'm1',
        description: 'Book the room',
      }),
    );
    expect(await screen.findByText('Book the room')).toBeInTheDocument();
    // The draft clears only after the create succeeds.
    expect(input).toHaveValue('');
  });

  it('keeps the typed draft when the create fails', async () => {
    stubInvoke({
      api_get_action_items: () => [makeItem()],
      api_create_action_item: () => {
        throw new Error('db locked');
      },
    });
    render(<ActionItemsSection meetingId="m1" hasSummary />);
    await screen.findByText('Send the deck to Alice');

    const input = screen.getByLabelText('Add an action item…');
    fireEvent.change(input, { target: { value: 'Book the room' } });
    fireEvent.submit(input);

    await waitFor(() => expect(toastError).toHaveBeenCalled());
    // The typed text survives the failure for retry.
    expect(input).toHaveValue('Book the room');
    expect(screen.queryByText('Book the room')).not.toBeInTheDocument();
  });

  it('reverts only the mutated item on failure, preserving a concurrent reload', async () => {
    const a = makeItem({ id: 'a', description: 'Item A' });
    const b = makeItem({ id: 'b', description: 'Item B' });
    let loads = 0;
    let rejectStatus!: (err: Error) => void;
    stubInvoke({
      // First load has only A; the reload (fired mid-mutation) also brings B.
      api_get_action_items: () => (++loads === 1 ? [a] : [a, b]),
      api_set_action_item_status: () =>
        new Promise((_resolve, reject) => {
          rejectStatus = reject;
        }),
    });
    const captured = captureUpdatedHandler();
    render(<ActionItemsSection meetingId="m1" hasSummary />);
    await screen.findByText('Item A');

    // Start completing A — the IPC call hangs.
    fireEvent.click(screen.getByRole('checkbox', { name: 'Mark "Item A" as completed' }));
    // A background extraction commits meanwhile and reloads the list with B.
    captured.handler!({ payload: { meeting_id: 'm1' } });
    await screen.findByText('Item B');

    // The status call now fails: only A reverts; B must survive (a whole-list
    // snapshot revert would clobber it).
    rejectStatus(new Error('nope'));
    await waitFor(() => expect(toastError).toHaveBeenCalled());
    expect(screen.getByText('Item B')).toBeInTheDocument();
    expect(
      screen.getByRole('checkbox', { name: 'Mark "Item A" as completed' }),
    ).toBeInTheDocument(); // back to open
  });

  it('shows the pre-summary empty state (no add row, no scan)', async () => {
    stubInvoke({ api_get_action_items: () => [] });
    render(<ActionItemsSection meetingId="m1" hasSummary={false} />);

    expect(
      await screen.findByText('Action items appear after the summary is generated.'),
    ).toBeInTheDocument();
    expect(screen.queryByText('Scan again')).not.toBeInTheDocument();
    expect(screen.queryByLabelText('Add an action item…')).not.toBeInTheDocument();
  });

  it('scans → toasts the inserted-or-updated count; the reload comes via the event', async () => {
    stubInvoke({
      api_get_action_items: () => [],
      api_extract_action_items: () => 2,
    });
    const captured = captureUpdatedHandler();
    render(<ActionItemsSection meetingId="m1" hasSummary />);

    fireEvent.click(await screen.findByText('Scan again'));

    await waitFor(() =>
      expect(callsFor('api_extract_action_items')[0]![1]).toEqual({ meetingId: 'm1' }),
    );
    await waitFor(() =>
      expect(toastSuccess).toHaveBeenCalledWith('2 action items added or updated'),
    );
    // No explicit refetch — the ONE reload path is the `action-items-updated` event.
    expect(callsFor('api_get_action_items').length).toBe(1);
    captured.handler!({ payload: { meeting_id: 'm1' } });
    await waitFor(() => expect(callsFor('api_get_action_items').length).toBe(2));
  });

  it('toasts "No new action items" when the scan is a no-op', async () => {
    stubInvoke({
      api_get_action_items: () => [],
      api_extract_action_items: () => 0,
    });
    render(<ActionItemsSection meetingId="m1" hasSummary />);

    fireEvent.click(await screen.findByText('Scan again'));

    await waitFor(() => expect(toastSuccess).toHaveBeenCalledWith('No new action items'));
  });

  it('refreshes when the action-items-updated event fires for this meeting', async () => {
    stubInvoke({ api_get_action_items: () => [] });
    const captured = captureUpdatedHandler();
    render(<ActionItemsSection meetingId="m1" hasSummary />);

    await waitFor(() => expect(captured.handler).not.toBeNull());
    const initialLoads = callsFor('api_get_action_items').length;

    // Another meeting's extraction is ignored…
    captured.handler!({ payload: { meeting_id: 'other' } });
    // …ours triggers a refresh.
    captured.handler!({ payload: { meeting_id: 'm1' } });

    await waitFor(() =>
      expect(callsFor('api_get_action_items').length).toBe(initialLoads + 1),
    );
  });
});
