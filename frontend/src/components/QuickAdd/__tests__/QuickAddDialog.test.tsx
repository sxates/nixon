import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { QuickAddDialog } from '../QuickAddDialog';

const invokeMock = vi.fn();
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...a: unknown[]) => invokeMock(...a) }));
vi.mock('next/navigation', () => ({ useRouter: () => ({ push: vi.fn() }) }));
vi.mock('sonner', () => ({ toast: Object.assign(vi.fn(), { success: vi.fn(), error: vi.fn() }) }));
vi.mock('@/components/ActionItems/MentionInput', () => ({
  MentionInput: (p: { ariaLabel?: string; onSubmit: (v: string) => void; onChange?: (v: string) => void }) => (
    <input
      aria-label={p.ariaLabel}
      onChange={(e) => p.onChange?.(e.target.value)}
      onKeyDown={(e) => e.key === 'Enter' && p.onSubmit((e.target as HTMLInputElement).value)}
    />
  ),
}));

const live = { id: 'm-live', title: 'Weekly sync' };

describe('QuickAddDialog', () => {
  beforeEach(() => {
    vi.useFakeTimers({ toFake: ['Date'], now: new Date(2026, 10, 12, 9, 0, 0) });
    invokeMock.mockReset();
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === 'api_get_meeting_notes') return { notesMarkdown: null, notesJson: '<p>Earlier</p>' };
      return null;
    });
  });

  const enter = (value: string) => {
    const field = screen.getByLabelText('Quick add To-Do');
    fireEvent.change(field, { target: { value } });
    fireEvent.keyDown(field, { key: 'Enter', target: { value } });
  };

  it('links to the live meeting and writes nothing into its notes (owner feedback, v0.18)', async () => {
    const onOpenChange = vi.fn();
    render(<QuickAddDialog open onOpenChange={onOpenChange} liveMeeting={live} />);
    expect(screen.getByRole('checkbox', { name: 'Add to “Weekly sync”' })).toBeInTheDocument();
    enter('Send the deck (Nov 20)');

    await waitFor(() => expect(onOpenChange).toHaveBeenCalledWith(false));
    expect(invokeMock).toHaveBeenCalledWith('api_create_action_item', {
      meetingId: 'm-live',
      description: 'Send the deck',
      dueDate: '2026-11-20',
    });
    expect(invokeMock).not.toHaveBeenCalledWith('api_get_meeting_notes', expect.anything());
    expect(invokeMock).not.toHaveBeenCalledWith('api_save_meeting_notes', expect.anything());
  });

  it('unticking "Add to …" makes a standalone to-do and leaves the notes alone', async () => {
    render(<QuickAddDialog open onOpenChange={vi.fn()} liveMeeting={live} />);
    fireEvent.click(screen.getByRole('checkbox'));
    enter('Call Sam');
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('api_create_action_item', {
        meetingId: null,
        description: 'Call Sam',
        dueDate: null,
      }),
    );
    expect(invokeMock).not.toHaveBeenCalledWith('api_get_meeting_notes', expect.anything());
  });
});
