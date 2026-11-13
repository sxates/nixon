import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { QuickAddDialog } from '../QuickAddDialog';
import { APPEND_NOTE_LINE_EVENT, type AppendNoteLineDetail } from '@/lib/notes-append';

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
    const field = screen.getByLabelText('Quick add to-do');
    fireEvent.change(field, { target: { value } });
    fireEvent.keyDown(field, { key: 'Enter', target: { value } });
  };

  it('links to the live meeting and hands the notes line to an open notepad', async () => {
    const seen: AppendNoteLineDetail[] = [];
    const take = (e: Event) => {
      const d = (e as CustomEvent<AppendNoteLineDetail>).detail;
      seen.push({ ...d });
      d.handled = true;
    };
    window.addEventListener(APPEND_NOTE_LINE_EVENT, take);
    const onOpenChange = vi.fn();
    render(<QuickAddDialog open onOpenChange={onOpenChange} liveMeeting={live} />);
    enter('Send the deck (Nov 20)');

    await waitFor(() => expect(seen).toHaveLength(1));
    expect(invokeMock).toHaveBeenCalledWith('api_create_action_item', {
      meetingId: 'm-live',
      description: 'Send the deck',
      dueDate: '2026-11-20',
    });
    expect(seen[0]).toMatchObject({ meetingId: 'm-live', line: '☐ To-do: Send the deck (due Next Friday)' });
    // The open notepad took it: no direct notes write that its autosave would clobber.
    expect(invokeMock).not.toHaveBeenCalledWith('api_save_meeting_notes', expect.anything());
    expect(onOpenChange).toHaveBeenCalledWith(false);
    window.removeEventListener(APPEND_NOTE_LINE_EVENT, take);
  });

  it('with no notepad open, appends the line to the stored notes', async () => {
    render(<QuickAddDialog open onOpenChange={vi.fn()} liveMeeting={live} />);
    enter('Call Sam');
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('api_save_meeting_notes', {
        meetingId: 'm-live',
        notesMarkdown: expect.stringContaining('☐ To-do: Call Sam'),
        notesJson: '<p>Earlier</p><p>☐ To-do: Call Sam</p>',
      }),
    );
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
