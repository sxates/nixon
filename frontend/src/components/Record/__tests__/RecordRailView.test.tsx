import React from 'react';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import userEvent from '@testing-library/user-event';

// The Transcript / Focus switch (owner feedback, v0.18) restyles the notes column; it must
// not remount the REAL notepad. Text typed just before a switch is still in the same editor
// node afterwards and reaches the debounced autosave. In Transcript the rail also carries the
// Topics row and a To-Dos tab; in Focus both live in the To-Do sidebar instead.

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
  MeetingTopicsRow: ({ meetingId, title }: { meetingId: string; title: string | null }) => (
    <div data-testid="topics-row">
      topics for {meetingId} ({title ?? 'untitled'})
    </div>
  ),
}));
vi.mock('@/components/Record/RecordTodoRail', () => ({
  RecordTodos: ({ meetingId, heading }: { meetingId: string; heading?: boolean }) => (
    <div data-testid="record-todos" data-heading={String(heading)}>
      todos for {meetingId}
    </div>
  ),
}));

import { RecordRail } from '@/components/Record/RecordRail';

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
    // The tab strip is the record strip height, so in Focus its rule meets the Topics strip's.
    expect(screen.getByTestId('record-rail-strip').className).toContain('min-h-11');
    rerender(<RecordRail meetingId="m-1" view="focus" />);
    expect(root().className).toContain('flex-1');
    expect(root().className).not.toContain('w-[40%]');
    await screen.findByRole('textbox');
  });

  it('Transcript: Topics row above the tabs and a To-Dos tab; Focus: neither', async () => {
    const user = userEvent.setup();
    const { rerender } = render(<RecordRail meetingId="m-1" view="transcript" meetingTitle="Weekly sync" />);
    expect(screen.getByTestId('topics-row')).toHaveTextContent('topics for m-1 (Weekly sync)');
    expect(screen.getAllByRole('tab').map((t) => t.textContent)).toEqual(['My Notes', 'Agenda', 'To-Dos']);
    // The Topics row sits above the tab strip.
    expect(
      screen.getByTestId('topics-row').compareDocumentPosition(screen.getByRole('tablist')) &
        Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy();
    expect(screen.queryByRole('button', { name: /add to-do/i })).toBeNull();

    await user.click(screen.getByRole('tab', { name: 'To-Dos' }));
    expect(screen.getByTestId('record-todos')).toHaveAttribute('data-heading', 'false');
    expect(screen.getByTestId('record-todos').closest('[role="tabpanel"]')).not.toHaveAttribute('hidden');
    // The notepad stays mounted behind the To-Dos tab.
    const editor = await screen.findByRole('textbox', { hidden: true });

    // Focus: My Notes / Agenda only; a rail left on To-Dos falls back to My Notes.
    rerender(<RecordRail meetingId="m-1" view="focus" meetingTitle="Weekly sync" />);
    expect(screen.queryByTestId('topics-row')).toBeNull();
    expect(screen.getAllByRole('tab').map((t) => t.textContent)).toEqual(['My Notes', 'Agenda']);
    expect(screen.getByRole('tab', { name: 'My Notes' })).toHaveAttribute('aria-selected', 'true');
    expect(screen.queryByTestId('record-todos')).toBeNull();

    // Back to Transcript: the To-Dos tab is where it was left.
    rerender(<RecordRail meetingId="m-1" view="transcript" meetingTitle="Weekly sync" />);
    expect(screen.getByRole('tab', { name: 'To-Dos' })).toHaveAttribute('aria-selected', 'true');
    expect(screen.getByRole('textbox', { hidden: true })).toBe(editor);
  });

  it('Focus: the tab strip carries the given keys at its far right, on the same rule', async () => {
    render(<RecordRail meetingId="m-1" view="focus" tabsTrailing={<button type="button">Participants</button>} />);
    const strip = screen.getByTestId('record-rail-strip');
    expect(strip).toContainElement(screen.getByRole('button', { name: 'Participants' }));
    expect(strip).toContainElement(screen.getByRole('tablist'));
    await screen.findByRole('textbox');
  });

  it('hides the Topics row before the meeting row exists', async () => {
    render(<RecordRail meetingId={null} view="transcript" />);
    expect(screen.queryByTestId('topics-row')).toBeNull();
    expect(screen.queryByRole('tab', { name: 'To-Dos' })).toBeNull();
    await screen.findByRole('textbox');
  });
});
