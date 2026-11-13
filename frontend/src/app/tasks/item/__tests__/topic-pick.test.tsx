import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import TaskItemPage from '@/app/tasks/item/page';

// specs/0089 — a topic picked in the detail title is pending until the title saves; Esc
// (cancel) throws it away so a later save doesn't apply a pick the owner abandoned.

vi.mock('next/navigation', () => ({
  useRouter: () => ({ push: vi.fn(), replace: vi.fn() }),
  useSearchParams: () => new URLSearchParams('id=ai-1'),
}));
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => {}) }));
vi.mock('@/hooks/useTopics', () => ({
  useTopics: () => ({
    data: [{ id: 'topic-1', name: 'Atlas CRM rebuild' }],
    loading: false,
    error: null,
    refresh: async () => {},
  }),
}));
vi.mock('sonner', () => ({ toast: Object.assign(vi.fn(), { success: vi.fn(), error: vi.fn() }) }));

import { invoke } from '@tauri-apps/api/core';
const invokeMock = vi.mocked(invoke);

const item = {
  id: 'ai-1', meetingId: null, description: 'Send the deck', status: 'open', source: 'manual',
  triage: 'listed', dueDate: null, notesMarkdown: null, topicId: null, topicName: null,
};

beforeEach(() => {
  invokeMock.mockReset();
  invokeMock.mockImplementation((async (cmd: string) => {
    if (cmd === 'api_get_action_item_detail') return { item, sources: [], personIds: [] };
    if (cmd === 'api_list_people_ranked' || cmd === 'api_list_people') return [];
    if (cmd === 'api_update_action_item') return item;
    return null;
  }) as unknown as typeof invoke);
});

describe('detail title topic pick', () => {
  it('Esc discards a pending @topic so the next save does not tag it', async () => {
    render(<TaskItemPage />);
    const title = (await screen.findByLabelText('To-do title')) as HTMLTextAreaElement;
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('api_list_people_ranked'));
    await Promise.resolve();
    fireEvent.change(title, { target: { value: 'Send the deck @Atl' } });
    fireEvent.keyDown(title, { key: 'Enter' }); // picks the topic
    expect(title.value).toBe('Send the deck ');
    fireEvent.keyDown(title, { key: 'Escape' }); // cancels the edit
    fireEvent.change(title, { target: { value: 'Send the deck now' } });
    fireEvent.keyDown(title, { key: 'Enter' }); // saves
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('api_update_action_item', expect.anything()));
    expect(invokeMock).not.toHaveBeenCalledWith('api_set_action_item_topic', expect.anything());
  });
});
