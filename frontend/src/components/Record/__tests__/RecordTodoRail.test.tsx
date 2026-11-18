import { fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import userEvent from '@testing-library/user-event';
import type { Topic, TodoRow } from '@/types';

// Focus view's To-Do rail (owner feedback, v0.18): the live Meeting's Topic To-Dos when it
// is on a Topic (a picker for several), else your To-Dos with no Topic; the add field creates
// the To-Do on the live Meeting and tags it to the Topic shown, and never writes to the notes.

const invokeMock = vi.fn();
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...a: unknown[]) => invokeMock(...a) }));
vi.mock('@/lib/safe-listen', () => ({ safeListen: () => () => {} }));
vi.mock('next/navigation', () => ({ useRouter: () => ({ push: vi.fn() }) }));
vi.mock('sonner', () => ({ toast: Object.assign(vi.fn(), { success: vi.fn(), error: vi.fn() }) }));

import { RecordTodoRail, RecordTodos } from '../RecordTodoRail';

const topic = (id: string, name: string, status: 'open' | 'closed' = 'open') =>
  ({ id, name, status, description: '' }) as unknown as Topic;
const row = (id: string, description: string, topicId: string | null, status = 'open'): TodoRow =>
  ({ id, description, status, topicId, sortOrder: null, source: 'manual', triage: 'listed', sourceCount: 0 }) as unknown as TodoRow;

let meetingTopics: Topic[];
// A tiny backend: created rows land in the list, tagging moves them to the topic.
let store: TodoRow[];

beforeEach(() => {
  invokeMock.mockReset();
  meetingTopics = [];
  store = [
    row('l1', 'Loose end', null),
    row('l2', 'Tagged elsewhere', 'other'),
    row('a1', 'Draft the plan', 't1'),
    row('a2', 'Old done one', 't1', 'completed'),
    row('b1', 'Review the budget', 't2'),
  ];
  invokeMock.mockImplementation(async (cmd: string, args?: Record<string, unknown>) => {
    switch (cmd) {
      case 'api_topics_for_meeting':
        return meetingTopics;
      case 'api_get_topic':
        return { topic: { id: args?.id }, todos: store.filter((r) => r.topicId === args?.id) };
      case 'api_list_todo_view':
        return store.filter((r) => r.status === 'open');
      case 'api_create_action_item': {
        const created = row('new-1', String(args?.description), null);
        store.push(created);
        return created;
      }
      case 'api_set_action_item_topic':
        store = store.map((r) => (r.id === args?.id ? { ...r, topicId: args?.topicId as string } : r));
        return null;
      default:
        return [];
    }
  });
});

describe('RecordTodoRail', () => {
  it('before the meeting row exists, shows a quiet empty state and loads nothing', () => {
    render(<RecordTodoRail meetingId={null} />);
    expect(screen.getByText(/show here once recording starts/i)).toBeInTheDocument();
    expect(screen.queryByLabelText(/add a to-do/i)).toBeNull();
    expect(invokeMock).not.toHaveBeenCalledWith('api_topics_for_meeting', expect.anything());
  });

  it('with no Topic, lists the To-Do list items that have no Topic', async () => {
    render(<RecordTodoRail meetingId="m-1" />);
    expect(await screen.findByText('Loose end')).toBeInTheDocument();
    expect(screen.queryByText('Tagged elsewhere')).toBeNull();
    expect(screen.getByText('Your To-Dos with no Topic')).toBeInTheDocument();
    expect(invokeMock).toHaveBeenCalledWith('api_list_todo_view', { view: 'list' });
    expect(invokeMock).not.toHaveBeenCalledWith('api_get_topic', expect.anything());
  });

  it("on a Topic, lists that Topic's open To-Dos (closed Topics are skipped)", async () => {
    meetingTopics = [topic('tx', 'Archived', 'closed'), topic('t1', 'Launch')];
    render(<RecordTodoRail meetingId="m-1" />);
    expect(await screen.findByText('Draft the plan')).toBeInTheDocument();
    expect(screen.queryByText('Old done one')).toBeNull();
    expect(screen.queryByText('Loose end')).toBeNull();
    expect(screen.getByText('From the Topic Launch')).toBeInTheDocument();
    expect(invokeMock).toHaveBeenCalledWith('api_get_topic', { id: 't1' });
    expect(invokeMock).not.toHaveBeenCalledWith('api_get_topic', { id: 'tx' });
  });

  it('with several Topics, a picker defaults to the first and switches the list', async () => {
    meetingTopics = [topic('t1', 'Launch'), topic('t2', 'Budget')];
    render(<RecordTodoRail meetingId="m-1" />);
    expect(await screen.findByText('Draft the plan')).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Topic: Launch' }));
    await userEvent.click(screen.getByRole('menuitemradio', { name: 'Budget' }));
    expect(await screen.findByText('Review the budget')).toBeInTheDocument();
    expect(screen.queryByText('Draft the plan')).toBeNull();
  });

  it('adds on the live Meeting, tags the Topic shown, parses the due date, and writes no notes line', async () => {
    meetingTopics = [topic('t1', 'Launch')];
    render(<RecordTodoRail meetingId="m-1" />);
    await screen.findByText('Draft the plan');

    const field = screen.getByLabelText('Add a To-Do to this Meeting and Launch');
    fireEvent.change(field, { target: { value: 'Book the kickoff (2030-01-04)' } });
    fireEvent.keyDown(field, { key: 'Enter' });

    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('api_set_action_item_topic', { id: 'new-1', topicId: 't1' }),
    );
    const create = invokeMock.mock.calls.find((c) => c[0] === 'api_create_action_item');
    expect(create?.[1]).toMatchObject({ meetingId: 'm-1', description: 'Book the kickoff', dueDate: '2030-01-04' });
    expect(invokeMock).not.toHaveBeenCalledWith('api_save_meeting_notes', expect.anything());
    const list = screen.getByRole('listbox', { name: 'Launch To-Dos' });
    expect(await within(list).findByText('Book the kickoff')).toBeInTheDocument();
  });

  it('with no Topic, adds on the live Meeting without tagging', async () => {
    render(<RecordTodoRail meetingId="m-1" />);
    await screen.findByText('Loose end');
    const field = screen.getByLabelText('Add a To-Do to this Meeting');
    fireEvent.change(field, { target: { value: 'Send notes' } });
    fireEvent.keyDown(field, { key: 'Enter' });
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('api_create_action_item', { meetingId: 'm-1', description: 'Send notes' }),
    );
    expect(invokeMock).not.toHaveBeenCalledWith('api_set_action_item_topic', expect.anything());
    expect(await screen.findByText('Send notes')).toBeInTheDocument();
  });

  it('Focus sidebar: the Topics row sits above the To-Dos', async () => {
    render(<RecordTodoRail meetingId="m-1" meetingTitle="Weekly sync" />);
    const topicsLabel = screen.getByText('Topics');
    const heading = screen.getByRole('heading', { name: 'To-Dos' });
    expect(topicsLabel.compareDocumentPosition(heading) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(screen.getByRole('button', { name: /Add to Topic/ })).toBeInTheDocument();
    await screen.findByText('Loose end');
  });

  it('as the Transcript rail\'s tab: the same list without its own heading or Topics row', async () => {
    render(<RecordTodos meetingId="m-1" heading={false} />);
    expect(await screen.findByText('Loose end')).toBeInTheDocument();
    expect(screen.queryByRole('heading', { name: 'To-Dos' })).toBeNull();
    expect(screen.queryByText('Topics')).toBeNull();
    expect(screen.getByLabelText('Add a To-Do to this Meeting')).toBeInTheDocument();
  });

  it('is the Transcript rail\'s width, and its Topics strip shares the record strip height', async () => {
    const { container } = render(<RecordTodoRail meetingId="m-1" />);
    const aside = screen.getByRole('complementary', { name: 'To-Dos' });
    expect(aside.className).toContain('w-[40%]');
    expect(aside.className).toContain('max-w-[440px]');
    expect(container.querySelector('.border-b.min-h-11')).not.toBeNull();
    await screen.findByText('Loose end');
  });
});
