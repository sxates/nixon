import React from 'react';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

// The Transcript / Focus switch (owner feedback, v0.18) restyles the notes column; it must
// not remount the REAL notepad. Text typed just before a switch is still in the same editor
// node afterwards and reaches the debounced autosave, and the Topics row + Add To-Do key
// render once a meeting row exists.

const saves: Array<{ meetingId: string; notesMarkdown: string | null }> = [];
vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn((cmd: string, args: { meetingId: string; notesMarkdown: string | null }) => {
    if (cmd === 'api_save_meeting_notes') saves.push(args);
    return Promise.resolve(null);
  }),
}));
vi.mock('sonner', () => ({ toast: { error: vi.fn(), success: vi.fn() } }));
vi.mock('@/components/Sidebar/SidebarProvider', () => ({ useSidebar: () => ({ currentMeeting: null }) }));
vi.mock('@/components/MeetingDetails/PrepPanel', () => ({ PrepPanel: () => <div /> }));
vi.mock('@/components/Record/RecordAgendaPanel', () => ({ RecordAgendaPanel: () => null }));
vi.mock('@/hooks/usePrepAvailability', () => ({
  usePrepAvailability: () => ({ hasPrep: false, openItemCount: 0, refresh: vi.fn() }),
}));
vi.mock('@/components/Topics/MeetingTopicsRow', () => ({
  MeetingTopicsRow: ({ meetingId, title, trailing }: { meetingId: string; title: string | null; trailing?: React.ReactNode }) => (
    <div data-testid="topics-row">
      topics for {meetingId} ({title ?? 'untitled'}){trailing}
    </div>
  ),
}));

import { RecordRail } from '@/components/Record/RecordRail';
import { OPEN_QUICK_ADD_EVENT } from '@/components/QuickAdd/QuickAddShortcut';

afterEach(() => {
  saves.length = 0;
  vi.clearAllMocks();
});

describe('RecordRail across the view switch', () => {
  it('keeps the same notepad editor, and the text typed just before the switch is saved', async () => {
    const { rerender, container } = render(<RecordRail meetingId="m-1" view="transcript" />);
    const editor = await screen.findByRole('textbox');
    // Let the initial (empty) load settle before typing.
    await waitFor(() => expect(editor).toHaveAttribute('contenteditable', 'true'));

    editor.innerHTML = '<p>said just before switching</p>';
    fireEvent.input(editor);
    rerender(<RecordRail meetingId="m-1" view="focus" />);

    expect(container.querySelector('[data-record-view]')).toHaveAttribute('data-record-view', 'focus');
    // Same node: no remount, so undo history and caret live on.
    expect(screen.getByRole('textbox')).toBe(editor);
    expect(editor.innerHTML).toContain('said just before switching');

    await waitFor(
      () => expect(saves.some((s) => s.meetingId === 'm-1' && s.notesMarkdown?.includes('said just before switching'))).toBe(true),
      { timeout: 3000 },
    );

    rerender(<RecordRail meetingId="m-1" view="transcript" />);
    expect(screen.getByRole('textbox')).toBe(editor);
  });

  it('Focus takes the full width; Transcript is the 40% rail', async () => {
    const { rerender, container } = render(<RecordRail meetingId="m-1" view="transcript" />);
    const root = () => container.querySelector('[data-record-view]') as HTMLElement;
    expect(root().className).toContain('w-[40%]');
    rerender(<RecordRail meetingId="m-1" view="focus" />);
    expect(root().className).toContain('flex-1');
    expect(root().className).not.toContain('w-[40%]');
    await screen.findByRole('textbox');
  });

  it('shows the Topics row with Add To-Do in Transcript, which opens the quick-add dialog', async () => {
    const opened = vi.fn();
    window.addEventListener(OPEN_QUICK_ADD_EVENT, opened);
    const { rerender } = render(<RecordRail meetingId="m-1" view="transcript" meetingTitle="Weekly sync" />);
    expect(screen.getByTestId('topics-row')).toHaveTextContent('topics for m-1 (Weekly sync)');
    fireEvent.click(screen.getByRole('button', { name: /add to-do/i }));
    expect(opened).toHaveBeenCalledTimes(1);

    // Focus: the Topics row stays, the To-Do rail's own field replaces the key.
    rerender(<RecordRail meetingId="m-1" view="focus" meetingTitle="Weekly sync" />);
    expect(screen.getByTestId('topics-row')).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: /add to-do/i })).toBeNull();
    window.removeEventListener(OPEN_QUICK_ADD_EVENT, opened);
    await screen.findByRole('textbox');
  });

  it('hides the Topics row before the meeting row exists', async () => {
    render(<RecordRail meetingId={null} view="transcript" />);
    expect(screen.queryByTestId('topics-row')).toBeNull();
    expect(screen.queryByRole('button', { name: /add to-do/i })).toBeNull();
    await screen.findByRole('textbox');
  });
});
