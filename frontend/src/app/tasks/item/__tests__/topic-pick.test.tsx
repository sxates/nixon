import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import type React from 'react';
import TaskItemPage from '@/app/tasks/item/page';

// specs/0089 — the detail page's Topic field, and a topic picked in the title.
//
// The detail payload is the REAL wire shape of the Rust `ActionItemDetail`: `topicId` /
// `topicName` sit beside `item`, not inside it (`ActionItem` has no topic field). Rust
// pins that shape in `tests/action_items_todo.rs::detail_wire_shape_carries_the_topic_beside_the_item`.

vi.mock('next/navigation', () => ({
  useRouter: () => ({ push: vi.fn(), replace: vi.fn() }),
  useSearchParams: () => new URLSearchParams('id=ai-1'),
}));
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => {}) }));
vi.mock('@/hooks/useTopics', () => ({
  useTopics: () => ({
    data: [
      { id: 'topic-1', name: 'Atlas CRM rebuild' },
      { id: 'topic-2', name: 'Beta launch' },
    ],
    loading: false,
    error: null,
    refresh: async () => {},
  }),
}));
vi.mock('sonner', () => ({ toast: Object.assign(vi.fn(), { success: vi.fn(), error: vi.fn() }) }));
// Radix Select can't be driven in jsdom; a native select keeps value/onValueChange/disabled.
vi.mock('@/components/ui/select', () => ({
  Select: ({
    value,
    onValueChange,
    children,
  }: {
    value: string;
    onValueChange: (v: string) => void;
    children: React.ReactNode;
  }) => (
    <select aria-label="Topic" value={value} onChange={(e) => onValueChange(e.target.value)}>
      {children}
    </select>
  ),
  SelectTrigger: () => null,
  SelectValue: () => null,
  SelectContent: ({ children }: { children: React.ReactNode }) => <>{children}</>,
  SelectItem: ({
    value,
    disabled,
    children,
  }: {
    value: string;
    disabled?: boolean;
    children: React.ReactNode;
  }) => (
    <option value={value} disabled={disabled}>
      {children}
    </option>
  ),
}));

import { invoke } from '@tauri-apps/api/core';
const invokeMock = vi.mocked(invoke);

const item = {
  id: 'ai-1', meetingId: null, description: 'Send the deck', status: 'open', source: 'manual',
  triage: 'listed', dueDate: null, notesMarkdown: null,
};
const NAMES: Record<string, string> = {
  'topic-1': 'Atlas CRM rebuild',
  'topic-2': 'Beta launch',
  'topic-9': 'Closed project',
};
let tagged: string | null = null;

beforeEach(() => {
  tagged = null;
  invokeMock.mockReset();
  invokeMock.mockImplementation((async (cmd: string, args?: Record<string, unknown>) => {
    if (cmd === 'api_get_action_item_detail')
      return {
        item,
        sources: [],
        personIds: [],
        topicId: tagged,
        topicName: tagged ? NAMES[tagged] : null,
      };
    if (cmd === 'api_set_action_item_topic') {
      tagged = (args?.topicId as string | null) ?? null;
      return null;
    }
    if (cmd === 'api_list_people_ranked' || cmd === 'api_list_people') return [];
    if (cmd === 'api_update_action_item') return item;
    return null;
  }) as unknown as typeof invoke);
});

const topicSelect = () => screen.findByLabelText('Topic') as Promise<HTMLSelectElement>;

describe('detail Topic field', () => {
  it('shows the tagged topic from the detail payload', async () => {
    tagged = 'topic-1';
    render(<TaskItemPage />);
    const select = await topicSelect();
    await waitFor(() => expect(select.value).toBe('topic-1'));
  });

  it('changes and clears the tag, and the field follows the reload', async () => {
    tagged = 'topic-1';
    render(<TaskItemPage />);
    const select = await topicSelect();
    await waitFor(() => expect(select.value).toBe('topic-1'));

    fireEvent.change(select, { target: { value: 'topic-2' } });
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('api_set_action_item_topic', { id: 'ai-1', topicId: 'topic-2' }),
    );
    await waitFor(() => expect(select.value).toBe('topic-2'));

    fireEvent.change(select, { target: { value: 'none' } });
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('api_set_action_item_topic', { id: 'ai-1', topicId: null }),
    );
    await waitFor(() => expect(select.value).toBe('none'));
  });

  it('keeps a closed topic visible as a disabled choice', async () => {
    tagged = 'topic-9';
    render(<TaskItemPage />);
    const select = await topicSelect();
    await waitFor(() => expect(select.value).toBe('topic-9'));
    const closed = screen.getByRole('option', { name: 'Closed project' }) as HTMLOptionElement;
    expect(closed.disabled).toBe(true);
  });
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

  it('does not re-tag when the picked topic is the one already tagged', async () => {
    tagged = 'topic-1';
    render(<TaskItemPage />);
    const select = await topicSelect();
    await waitFor(() => expect(select.value).toBe('topic-1'));
    const title = (await screen.findByLabelText('To-do title')) as HTMLTextAreaElement;
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('api_list_people_ranked'));
    await Promise.resolve();
    fireEvent.change(title, { target: { value: 'Send the deck now @Atl' } });
    fireEvent.keyDown(title, { key: 'Enter' }); // picks the already-tagged topic
    fireEvent.change(title, { target: { value: 'Send the deck now' } });
    fireEvent.keyDown(title, { key: 'Enter' }); // saves
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('api_update_action_item', expect.anything()));
    expect(invokeMock).not.toHaveBeenCalledWith('api_set_action_item_topic', expect.anything());
  });
});
