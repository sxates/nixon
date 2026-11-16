import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { TodoRow } from '@/types';

const invokeMock = vi.fn();
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...a: unknown[]) => invokeMock(...a) }));
vi.mock('@/lib/safe-listen', () => ({ safeListen: () => () => {} }));
const push = vi.fn();
vi.mock('next/navigation', () => ({ useRouter: () => ({ push }) }));
vi.mock('sonner', () => ({ toast: Object.assign(vi.fn(), { success: vi.fn(), error: vi.fn() }) }));

import { TopicTodosCard } from '../TopicTodosCard';

const row = (id: string, description: string, status = 'open', sortOrder: number | null = null): TodoRow =>
  ({
    id,
    description,
    status,
    sortOrder,
    source: 'manual',
    triage: 'listed',
    topicId: 'topic-1',
    sourceCount: 0,
  }) as unknown as TodoRow;

describe('TopicTodosCard', () => {
  beforeEach(() => {
    invokeMock.mockReset();
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === 'api_create_action_item') return row('new-1', 'Book the kickoff');
      if (cmd === 'api_list_people_ranked') return [];
      return { id: 'a1' };
    });
  });

  it('names the person an adopted to-do came from (not "Unknown")', async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === 'api_list_people') return [{ id: 'p1', displayName: 'Rafael Duarte' }];
      return [];
    });
    const adopted = { ...row('a1', 'Finalize the enclosure'), assigneePersonId: 'p1', assigneeIsSelf: false } as TodoRow;
    render(<TopicTodosCard topicId="topic-1" todos={[adopted]} onChanged={vi.fn()} />);
    expect(await screen.findByText(/adopted from Rafael Duarte/)).toBeInTheDocument();
    expect(screen.queryByText(/Unknown/)).toBeNull();
  });

  it('"Show in To-Do" is on by default and writes hideFromTodo as its inverse (specs/0090)', async () => {
    const onChanged = vi.fn();
    render(<TopicTodosCard topicId="topic-1" todos={[row('a1', 'Send the deck')]} onChanged={onChanged} />);
    const toggle = screen.getByRole('switch', { name: 'Show in To-Do' });
    expect(toggle).toHaveAttribute('aria-checked', 'true');
    expect(screen.queryByText('Not on your To-Do list')).toBeNull();
    fireEvent.click(toggle);
    expect(toggle).toHaveAttribute('aria-checked', 'false');
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('api_update_topic', {
        id: 'topic-1', name: null, description: null, status: null, starred: null, hideFromTodo: true,
      }),
    );
    await waitFor(() => expect(onChanged).toHaveBeenCalled());
  });

  it('a topic kept off the list shows the switch off; a failed change reverts it', async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === 'api_update_topic') throw new Error('nope');
      return [];
    });
    render(<TopicTodosCard topicId="topic-1" todos={[]} onChanged={vi.fn()} hideFromTodo />);
    const toggle = screen.getByRole('switch', { name: 'Show in To-Do' });
    expect(toggle).toHaveAttribute('aria-checked', 'false');
    fireEvent.click(toggle);
    await waitFor(() => expect(toggle).toHaveAttribute('aria-checked', 'false'));
    expect(invokeMock).toHaveBeenCalledWith('api_update_topic', expect.objectContaining({ hideFromTodo: false }));
  });

  it('reorders within its own places on the To-Do list (specs/0090)', async () => {
    const todos = [row('t1', 'First', 'open', 1), row('t2', 'Second', 'open', 3), row('t3', 'Third', 'open', 4)];
    render(<TopicTodosCard topicId="topic-1" todos={todos} onChanged={vi.fn()} />);
    expect(screen.getByText(/⌘↑↓ move/)).toBeTruthy();
    // Keyboard on a grip moves its row one place (the same path a drag drop takes).
    const grips = screen.getAllByRole('button', { name: 'Drag to reorder' });
    fireEvent.keyDown(grips[2], { key: 'ArrowUp' });
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('api_reorder_action_items_within', {
        orderedIds: ['t1', 't3', 't2'],
      }),
    );
    // Optimistic: Third now sits at Second's old place.
    const texts = screen.getAllByText(/^(First|Second|Third)$/).map((n) => n.textContent);
    expect(texts).toEqual(['First', 'Third', 'Second']);
  });

  it('titles the card and lists the items', () => {
    render(<TopicTodosCard topicId="topic-1" todos={[row('a1', 'Send the deck')]} onChanged={vi.fn()} />);
    expect(screen.getByText('My action items')).toBeTruthy();
    expect(screen.getByText('Send the deck')).toBeTruthy();
  });

  it('checking an item calls the same completion command as the To-Do list', async () => {
    const onChanged = vi.fn();
    render(<TopicTodosCard topicId="topic-1" todos={[row('a1', 'Send the deck')]} onChanged={onChanged} />);
    fireEvent.click(screen.getByRole('checkbox', { name: 'Mark "Send the deck" as completed' }));
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('api_set_action_item_status', {
        id: 'a1',
        status: 'completed',
      }),
    );
    await waitFor(() => expect(onChanged).toHaveBeenCalled());
  });

  it('quick add creates the item, then tags it to this Topic', async () => {
    render(<TopicTodosCard topicId="topic-1" todos={[]} onChanged={vi.fn()} />);
    const field = screen.getByLabelText('Add an action item to this Topic');
    fireEvent.change(field, { target: { value: 'Book the kickoff' } });
    fireEvent.keyDown(field, { key: 'Enter' });
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('api_set_action_item_topic', {
        id: 'new-1',
        topicId: 'topic-1',
      }),
    );
    expect(invokeMock).toHaveBeenCalledWith(
      'api_create_action_item',
      expect.objectContaining({ description: 'Book the kickoff', meetingId: null }),
    );
  });

  it('a refused tag still counts as added: warns, no duplicate-prone error', async () => {
    const { toast } = await import('sonner');
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === 'api_create_action_item') return row('new-1', 'Book the kickoff');
      if (cmd === 'api_list_people_ranked') return [];
      if (cmd === 'api_set_action_item_topic') throw new Error('That topic is closed.');
      return null;
    });
    const onChanged = vi.fn();
    render(<TopicTodosCard topicId="topic-1" todos={[]} onChanged={onChanged} />);
    const field = screen.getByLabelText('Add an action item to this Topic') as HTMLInputElement;
    fireEvent.change(field, { target: { value: 'Book the kickoff' } });
    fireEvent.keyDown(field, { key: 'Enter' });
    await waitFor(() =>
      expect(toast.error).toHaveBeenCalledWith('Added, but not to the Topic', expect.anything()),
    );
    expect(toast.error).not.toHaveBeenCalledWith('Could not add the item', expect.anything());
    await waitFor(() => expect(field.value).toBe(''));
    expect(onChanged).toHaveBeenCalled();
  });

  it('shows a person mention as the plain name', () => {
    render(
      <TopicTodosCard
        topicId="topic-1"
        todos={[row('a1', 'Ping @[Sam Rivera](person:p-1) about it')]}
        onChanged={vi.fn()}
      />,
    );
    expect(screen.getByText('@Sam Rivera')).toBeTruthy();
    expect(screen.queryByText(/person:p-1/)).toBeNull();
  });

  it('a ticked item stays on the card, struck through, with its dismiss button', async () => {
    render(<TopicTodosCard topicId="topic-1" todos={[row('a1', 'Send the deck')]} onChanged={vi.fn()} />);
    fireEvent.click(screen.getByRole('checkbox', { name: 'Mark "Send the deck" as completed' }));
    const title = await screen.findByRole('button', { name: 'Send the deck' });
    expect(title.className).toContain('line-through');
    expect(screen.getByRole('button', { name: 'Dismiss "Send the deck" from this Topic' })).toBeTruthy();
  });

  it('dismiss takes one done item off the Topic; Dismiss done takes them all', async () => {
    const onChanged = vi.fn();
    const todos = [
      row('a1', 'Send the deck', 'completed'),
      row('a2', 'Book the room', 'completed'),
      row('a3', 'Still open'),
    ];
    const { rerender } = render(<TopicTodosCard topicId="topic-1" todos={todos} onChanged={onChanged} />);
    fireEvent.click(screen.getByRole('button', { name: 'Dismiss "Send the deck" from this Topic' }));
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('api_clear_topic_todos', { topicId: 'topic-1', ids: ['a1'] }),
    );
    expect(screen.queryByText('Send the deck')).toBeNull();
    expect(screen.getByText('Still open')).toBeTruthy();

    rerender(<TopicTodosCard topicId="topic-1" todos={todos.slice(1)} onChanged={onChanged} />);
    fireEvent.click(screen.getByRole('button', { name: 'Dismiss done' }));
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('api_clear_topic_todos', { topicId: 'topic-1', ids: ['a2'] }),
    );
    expect(screen.queryByRole('button', { name: 'Dismiss done' })).toBeNull();
  });

  it('works from the keyboard like the To-Do list: arrow to select, Cmd-D to tick off', async () => {
    render(
      <TopicTodosCard
        topicId="topic-1"
        todos={[row('a1', 'Send the deck'), row('a2', 'Book the room')]}
        onChanged={vi.fn()}
      />,
    );
    const list = screen.getByRole('listbox', { name: "This Topic's action items" });
    list.focus();
    fireEvent.keyDown(list, { key: 'ArrowDown' });
    fireEvent.keyDown(list, { key: 'ArrowDown' });
    expect(screen.getByRole('option', { selected: true }).textContent).toContain('Book the room');
    fireEvent.keyDown(list, { key: 'd', metaKey: true });
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('api_set_action_item_status', { id: 'a2', status: 'completed' }),
    );
  });
});
