import { describe, it, expect, vi, beforeEach } from 'vitest';
import { act, renderHook, waitFor } from '@testing-library/react';
import { useTodoViews } from '@/hooks/useTodoViews';
import type { TodoRow, TodoView } from '@/types';

// specs/0088 — the To-do / Inbox / Done loader. Locks: the `action-items-updated`
// refetch, that only the latest request writes state (stale-overlap guard), the
// includeDismissed toggle, and that a failed load is surfaced rather than swallowed.

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn() }));

import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

const invokeMock = vi.mocked(invoke);
const listenMock = vi.mocked(listen);

function row(id: string): TodoRow {
  return {
    id,
    meetingId: null,
    description: id,
    assigneePersonId: null,
    assigneeIsSelf: false,
    assigneeRaw: null,
    dueHint: null,
    dueDate: null,
    status: 'open',
    source: 'manual',
    userEdited: false,
    contentKey: id,
    createdAt: '2026-07-01T10:00:00Z',
    updatedAt: '2026-07-01T10:00:00Z',
    completedAt: null,
    sortOrder: null,
    triage: 'listed',
    adoptedAt: null,
    notesMarkdown: null,
    meetingTitle: null,
    meetingCreatedAt: null,
    sourceCount: 0,
    latestSourceAt: null,
    latestContext: null,
  };
}

let handler: (() => void) | null = null;

beforeEach(() => {
  vi.clearAllMocks();
  handler = null;
  listenMock.mockImplementation((async (_name: string, cb: () => void) => {
    handler = cb;
    return () => {};
  }) as unknown as typeof listen);
});

function stub(byView: Partial<Record<TodoView, () => Promise<TodoRow[]> | TodoRow[]>> = {}, count = 0) {
  invokeMock.mockImplementation((async (cmd: string, args?: { view: TodoView }) => {
    if (cmd === 'api_inbox_count') return count;
    if (cmd === 'api_list_todo_view') return (byView[args!.view] ?? (() => []))();
    throw new Error(`unexpected ${cmd}`);
  }) as unknown as typeof invoke);
}

const viewCalls = (view: TodoView) =>
  invokeMock.mock.calls.filter(([c, a]) => c === 'api_list_todo_view' && (a as { view: string }).view === view);

describe('useTodoViews', () => {
  it('refetches when action-items-updated fires', async () => {
    let list = [row('a')];
    stub({ list: () => list }, 1);
    const { result } = renderHook(() => useTodoViews());
    await waitFor(() => expect(result.current.loaded).toBe(true));
    expect(result.current.list.map((r) => r.id)).toEqual(['a']);

    list = [row('a'), row('b')];
    await waitFor(() => expect(handler).not.toBeNull());
    await act(async () => handler!());
    await waitFor(() => expect(result.current.list.map((r) => r.id)).toEqual(['a', 'b']));
  });

  it('lets only the latest request write state (stale-overlap guard)', async () => {
    let release!: (rows: TodoRow[]) => void;
    let calls = 0;
    stub({
      list: () => {
        calls += 1;
        if (calls === 1) return new Promise<TodoRow[]>((r) => (release = r)); // slow first load
        return [row('fresh')];
      },
    });
    const { result } = renderHook(() => useTodoViews());
    await waitFor(() => expect(handler).not.toBeNull());
    await act(async () => handler!()); // second, faster load
    await waitFor(() => expect(result.current.list.map((r) => r.id)).toEqual(['fresh']));

    await act(async () => release([row('stale')])); // the slow one lands last
    expect(result.current.list.map((r) => r.id)).toEqual(['fresh']);
  });

  it('only loads the dismissed list when includeDismissed is on', async () => {
    stub({ dismissed: () => [row('gone')] });
    const { result, rerender } = renderHook(
      ({ inc }: { inc: boolean }) => useTodoViews({ includeDismissed: inc }),
      { initialProps: { inc: false } },
    );
    await waitFor(() => expect(result.current.loaded).toBe(true));
    expect(viewCalls('dismissed')).toHaveLength(0);
    expect(result.current.dismissed).toEqual([]);

    rerender({ inc: true });
    await waitFor(() => expect(result.current.dismissed.map((r) => r.id)).toEqual(['gone']));
    expect(viewCalls('dismissed')).toHaveLength(1);
  });

  it('surfaces a load failure as a friendly error, and clears it on the next success', async () => {
    let fail = true;
    stub({
      list: () => {
        if (fail) throw new Error('db locked');
        return [row('a')];
      },
    });
    const errSpy = vi.spyOn(console, 'error').mockImplementation(() => {});
    const { result } = renderHook(() => useTodoViews());
    await waitFor(() => expect(result.current.loaded).toBe(true));
    expect(result.current.error).toBe('Could not load your to-do list.');

    fail = false;
    await act(async () => result.current.reload());
    expect(result.current.error).toBeNull();
    expect(result.current.list).toHaveLength(1);
    errSpy.mockRestore();
  });
});

describe('useTodoViews countOnly (sidebar badge)', () => {
  it('fetches only the inbox count', async () => {
    stub({}, 3);
    const { result } = renderHook(() => useTodoViews({ countOnly: true }));
    await waitFor(() => expect(result.current.inboxCount).toBe(3));
    expect(invokeMock.mock.calls.filter(([c]) => c === 'api_list_todo_view')).toHaveLength(0);
  });
});
