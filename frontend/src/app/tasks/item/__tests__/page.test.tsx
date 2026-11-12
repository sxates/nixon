import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { render, screen, fireEvent, waitFor, act } from '@testing-library/react';
import TaskItemPage from '@/app/tasks/item/page';
import type { ActionItem, ActionItemDetail } from '@/types';

// specs/0088 — the to-do detail page. Locks: sources listed oldest first with their context
// line and a meeting link, Notes autosave (600 ms debounce, on blur, flush on unmount), the
// "no longer exists" empty state, and the state-appropriate triage actions.

const pushMock = vi.fn();
let searchParams = new URLSearchParams('id=ai-1');
vi.mock('next/navigation', () => ({
  useRouter: () => ({ push: pushMock, replace: vi.fn() }),
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

function item(overrides: Partial<ActionItem> = {}): ActionItem {
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
    ...overrides,
  };
}

const detail = (over: Partial<ActionItemDetail> = {}): ActionItemDetail => ({
  item: item(),
  sources: [
    {
      meetingId: 'm1',
      meetingTitle: 'Weekly sync',
      meetingCreatedAt: '2026-11-01T09:00:00Z',
      context: 'Dana asked for the deck before Friday',
      phrasing: 'Send the deck',
    },
    {
      meetingId: 'm2',
      meetingTitle: 'Planning review',
      meetingCreatedAt: '2026-11-08T09:00:00Z',
      context: 'Deck still outstanding',
      phrasing: 'Send the deck',
    },
  ],
  personIds: [],
  ...over,
});

function stub(d: ActionItemDetail | Error, extra: Record<string, (a?: any) => unknown> = {}) { // eslint-disable-line @typescript-eslint/no-explicit-any
  invokeMock.mockImplementation((async (cmd: string, args?: any) => { // eslint-disable-line @typescript-eslint/no-explicit-any
    if (cmd in extra) return extra[cmd]!(args);
    if (cmd === 'api_get_action_item_detail') {
      if (d instanceof Error) throw d;
      return d;
    }
    if (cmd === 'api_list_people') return [];
    if (cmd === 'api_list_people_ranked') return [{ id: 'p-1', displayName: 'Sam Rivera', email: null, role: null, notes: null, voiceprintOptOut: false, starred: false, createdAt: '', updatedAt: '' }];
    if (cmd === 'api_update_action_item') return item();
    if (cmd === 'api_set_action_item_notes') return null;
    throw new Error(`Unexpected command in test: ${cmd}`);
  }) as unknown as typeof invoke);
}

const callsTo = (cmd: string) => invokeMock.mock.calls.filter(([c]) => c === cmd);

beforeEach(() => {
  invokeMock.mockReset();
  pushMock.mockReset();
  searchParams = new URLSearchParams('id=ai-1');
  listenMock.mockReset();
  listenMock.mockResolvedValue(() => {});
});
afterEach(() => vi.useRealTimers());

describe('to-do detail page', () => {
  it('shows sources oldest first with context', async () => {
    stub(detail());
    render(<TaskItemPage />);
    await screen.findByText('Weekly sync');
    const titles = screen.getAllByText(/Weekly sync|Planning review/).map((n) => n.textContent);
    expect(titles).toEqual(['Weekly sync', 'Planning review']);
    expect(screen.getByText('Dana asked for the deck before Friday')).toBeTruthy();
    expect(screen.getByRole('heading', { name: 'Sources' })).toBeTruthy();
    fireEvent.click(screen.getByRole('button', { name: /Weekly sync/ }));
    expect(pushMock).toHaveBeenCalledWith('/meeting-details?id=m1');
  });

  it('notes autosave debounced', async () => {
    stub(detail());
    render(<TaskItemPage />);
    const box = (await screen.findByLabelText('Notes')) as HTMLTextAreaElement;
    vi.useFakeTimers();
    fireEvent.change(box, { target: { value: 'Draft' } });
    fireEvent.change(box, { target: { value: 'Draft notes' } });
    await act(async () => { vi.advanceTimersByTime(500); });
    expect(callsTo('api_set_action_item_notes')).toHaveLength(0);
    await act(async () => { vi.advanceTimersByTime(150); });
    expect(callsTo('api_set_action_item_notes')).toHaveLength(1);
    expect(callsTo('api_set_action_item_notes')[0][1]).toEqual({ id: 'ai-1', markdown: 'Draft notes' });
  });

  it('notes save on blur and flush on unmount', async () => {
    stub(detail());
    const { unmount } = render(<TaskItemPage />);
    const box = (await screen.findByLabelText('Notes')) as HTMLTextAreaElement;
    fireEvent.change(box, { target: { value: 'One' } });
    fireEvent.blur(box);
    await waitFor(() => expect(callsTo('api_set_action_item_notes')).toHaveLength(1));
    fireEvent.change(box, { target: { value: 'Two' } });
    unmount();
    expect(callsTo('api_set_action_item_notes')).toHaveLength(2);
    expect(callsTo('api_set_action_item_notes')[1][1]).toEqual({ id: 'ai-1', markdown: 'Two' });
  });

  it('empty notes are saved as null', async () => {
    stub(detail({ item: item({ notesMarkdown: 'Old' }) }));
    render(<TaskItemPage />);
    const box = (await screen.findByLabelText('Notes')) as HTMLTextAreaElement;
    expect(box.value).toBe('Old');
    fireEvent.change(box, { target: { value: '  ' } });
    fireEvent.blur(box);
    await waitFor(() =>
      expect(callsTo('api_set_action_item_notes')[0][1]).toEqual({ id: 'ai-1', markdown: null }),
    );
  });

  it('unknown id shows empty state', async () => {
    stub(new Error('That action item no longer exists'));
    render(<TaskItemPage />);
    expect(await screen.findByText('This to-do no longer exists')).toBeTruthy();
  });

  it('offers actions that match the item state', async () => {
    stub(detail({ item: item({ triage: 'suggested' }) }));
    const { unmount } = render(<TaskItemPage />);
    await screen.findByRole('button', { name: 'Add to my list' });
    expect(screen.getByRole('button', { name: 'Dismiss' })).toBeTruthy();
    unmount();

    stub(detail({ item: item({ triage: 'record' }) }));
    const r2 = render(<TaskItemPage />);
    await screen.findByRole('button', { name: 'Adopt' });
    r2.unmount();

    stub(detail({ item: item({ status: 'dismissed' }) }));
    const r3 = render(<TaskItemPage />);
    await screen.findByRole('button', { name: 'Restore' });
    r3.unmount();

    stub(detail({ item: item({ status: 'completed' }) }));
    const r4 = render(<TaskItemPage />);
    await screen.findByRole('button', { name: 'Reopen' });
    r4.unmount();

    stub(detail());
    render(<TaskItemPage />);
    await screen.findByRole('checkbox', { name: /Mark .* as completed/ });
  });

  it('Add to my list triages then reloads', async () => {
    let triage: ActionItem['triage'] = 'suggested';
    stub(detail(), {
      api_get_action_item_detail: () => detail({ item: item({ triage }) }),
      api_triage_action_item: () => {
        triage = 'listed';
        return item({ triage });
      },
    });
    render(<TaskItemPage />);
    fireEvent.click(await screen.findByRole('button', { name: 'Add to my list' }));
    await screen.findByRole('checkbox', { name: /Mark .* as completed/ });
    expect(callsTo('api_triage_action_item')[0][1]).toEqual({ id: 'ai-1', action: 'add_to_list' });
  });

  it('has exactly one h1', async () => {
    stub(detail());
    render(<TaskItemPage />);
    await screen.findByLabelText('Notes');
    expect(screen.getAllByRole('heading', { level: 1 })).toHaveLength(1);
  });

  it('title editing picks a person and saves the token', async () => {
    stub(detail());
    render(<TaskItemPage />);
    const title = (await screen.findByLabelText('Title')) as HTMLInputElement;
    expect(title.value).toBe('Send the deck');
    fireEvent.change(title, { target: { value: 'Send the deck to @sa' } });
    fireEvent.click(await screen.findByText('Sam Rivera'));
    fireEvent.keyDown(title, { key: 'Enter' });
    await waitFor(() => expect(callsTo('api_update_action_item')).toHaveLength(1));
    expect(callsTo('api_update_action_item')[0][1]).toMatchObject({
      description: 'Send the deck to @[Sam Rivera](person:p-1)',
    });
  });

  it('a stale refresh landing after a notes save does not overwrite the notes', async () => {
    let listener: (() => void) | undefined;
    listenMock.mockImplementation((async (_e: string, cb: () => void) => { listener = cb; return () => {}; }) as unknown as typeof listen);
    let release: (() => void) | undefined;
    let calls = 0;
    stub(detail(), {
      api_get_action_item_detail: async () => {
        calls += 1;
        if (calls === 2) {
          await new Promise<void>((r) => { release = r; });
          return detail({ item: item({ notesMarkdown: 'OLD' }) });
        }
        return detail();
      },
    });
    render(<TaskItemPage />);
    const box = (await screen.findByLabelText('Notes')) as HTMLTextAreaElement;
    await waitFor(() => expect(listener).toBeTruthy());
    act(() => listener!());
    await waitFor(() => expect(release).toBeTruthy());
    fireEvent.change(box, { target: { value: 'Newer' } });
    fireEvent.blur(box);
    await waitFor(() => expect(callsTo('api_set_action_item_notes')).toHaveLength(1));
    await act(async () => { release!(); });
    expect((screen.getByLabelText('Notes') as HTMLTextAreaElement).value).toBe('Newer');
  });

  it('a failed refresh keeps the loaded page and toasts', async () => {
    let listener: (() => void) | undefined;
    listenMock.mockImplementation((async (_e: string, cb: () => void) => { listener = cb; return () => {}; }) as unknown as typeof listen);
    let fail = false;
    stub(detail(), {
      api_get_action_item_detail: () => {
        if (fail) throw new Error('boom');
        return detail();
      },
    });
    render(<TaskItemPage />);
    await screen.findByLabelText('Notes');
    await waitFor(() => expect(listener).toBeTruthy());
    fail = true;
    await act(async () => { listener!(); });
    await waitFor(() => expect(vi.mocked(toast.error)).toHaveBeenCalled());
    expect(screen.getByLabelText('Notes')).toBeTruthy();
    expect(screen.queryByText('Could not load this to-do')).toBeNull();
  });
});
