import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { QuickAddDialog } from '../QuickAddDialog';

const invokeMock = vi.fn();
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...a: unknown[]) => invokeMock(...a) }));
vi.mock('next/navigation', () => ({ useRouter: () => ({ push: vi.fn() }) }));
vi.mock('@/lib/safe-listen', () => ({ safeListen: () => () => {} }));
vi.mock('sonner', () => ({ toast: Object.assign(vi.fn(), { success: vi.fn(), error: vi.fn() }) }));

describe('quick add with @topic (specs/0089 acceptance line)', () => {
  beforeEach(() => {
    vi.useFakeTimers({ toFake: ['Date'], now: new Date(2026, 10, 12, 9, 0, 0) });
    invokeMock.mockReset();
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === 'api_list_people_ranked') return [];
      if (cmd === 'api_list_topics') {
        return [{ id: 'topic-1', name: 'Atlas CRM rebuild', status: 'open' }];
      }
      if (cmd === 'api_create_action_item') return { id: 'new-1' };
      return null;
    });
  });

  it('"Draft sync doc @Atl" + "(Nov 20)" creates the to-do, then tags it', async () => {
    render(<QuickAddDialog open onOpenChange={vi.fn()} liveMeeting={null} />);
    const field = screen.getByLabelText('Quick add to-do') as HTMLInputElement;
    await waitFor(() =>
      expect(invokeMock.mock.calls.some((c) => c[0] === 'api_list_topics')).toBe(true),
    );
    await waitFor(() =>
      expect(invokeMock.mock.calls.some((c) => c[0] === 'api_list_people_ranked')).toBe(true),
    );
    await Promise.resolve();
    fireEvent.change(field, { target: { value: 'Draft sync doc @Atl' } });
    await screen.findByText('Atlas CRM rebuild', { selector: '[role="option"]' });
    fireEvent.keyDown(field, { key: 'Enter' });
    expect(field.value).toBe('Draft sync doc ');
    fireEvent.change(field, { target: { value: 'Draft sync doc (Nov 20)' } });
    fireEvent.keyDown(field, { key: 'Enter' });

    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('api_set_action_item_topic', {
        id: 'new-1',
        topicId: 'topic-1',
      }),
    );
    expect(invokeMock).toHaveBeenCalledWith('api_create_action_item', {
      meetingId: null,
      description: 'Draft sync doc',
      dueDate: '2026-11-20',
    });
  });
});
