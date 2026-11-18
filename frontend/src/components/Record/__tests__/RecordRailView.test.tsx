import React from 'react';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import userEvent from '@testing-library/user-event';

// The Transcript · My Notes · Agenda views (owner feedback, v0.18) restyle this column; they
// must not remount the REAL notepad. Text typed just before a switch is still in the same
// editor node afterwards and reaches the debounced autosave. In Transcript the rail starts
// with its My Notes / Agenda / To-Dos tabs; My Notes and Agenda show one panel with no
// sub-tabs under the page's meeting bar, and there is only ever one PrepPanel.

const saves: Array<{ meetingId: string; notesMarkdown: string | null }> = [];
vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn((cmd: string, args: { meetingId: string; notesMarkdown: string | null }) => {
    if (cmd === 'api_save_meeting_notes') saves.push(args);
    return Promise.resolve(null);
  }),
}));
vi.mock('sonner', () => ({ toast: { error: vi.fn(), success: vi.fn() } }));
vi.mock('@/components/Sidebar/SidebarProvider', () => ({ useSidebar: () => ({ currentMeeting: null }) }));
const prepMounts = { count: 0 };
vi.mock('@/components/MeetingDetails/PrepPanel', async () => {
  const { useEffect } = await import('react');
  return {
    PrepPanel: () => {
      useEffect(() => {
        prepMounts.count += 1;
      }, []);
      return <div data-testid="prep-panel" />;
    },
  };
});
vi.mock('@/components/Record/RecordAgendaPanel', () => ({ RecordAgendaPanel: () => null }));
vi.mock('@/hooks/usePrepAvailability', () => ({
  usePrepAvailability: () => ({ hasPrep: false, openItemCount: 0, refresh: vi.fn() }),
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
  prepMounts.count = 0;
  vi.clearAllMocks();
});

describe('RecordRail across the views', () => {
  it('keeps the same notepad editor through all three views, and the text typed just before a switch is saved', async () => {
    const { rerender, container } = render(<RecordRail meetingId="m-1" view="transcript" />);
    const editor = await screen.findByRole('textbox');
    // Let the initial (empty) load settle before typing.
    await waitFor(() => expect(editor).toHaveAttribute('contenteditable', 'true'));

    editor.innerHTML = '<p>said just before switching</p>';
    fireEvent.input(editor);
    rerender(<RecordRail meetingId="m-1" view="notes" />);

    expect(container.querySelector('[data-record-view]')).toHaveAttribute('data-record-view', 'notes');
    // Same node: no remount, so undo history and caret live on.
    expect(screen.getByRole('textbox')).toBe(editor);
    expect(editor.innerHTML).toContain('said just before switching');

    await waitFor(
      () => expect(saves.some((s) => s.meetingId === 'm-1' && s.notesMarkdown?.includes('said just before switching'))).toBe(true),
      { timeout: 3000 },
    );

    // Agenda hides the notepad but keeps it; back through My Notes and Transcript, same node.
    rerender(<RecordRail meetingId="m-1" view="agenda" />);
    expect(screen.getByRole('textbox', { hidden: true })).toBe(editor);
    rerender(<RecordRail meetingId="m-1" view="notes" />);
    expect(screen.getByRole('textbox')).toBe(editor);
    rerender(<RecordRail meetingId="m-1" view="transcript" />);
    expect(screen.getByRole('textbox')).toBe(editor);
  });

  it('My Notes / Agenda take the full width; Transcript is the 40% rail', async () => {
    const { rerender, container } = render(<RecordRail meetingId="m-1" view="transcript" />);
    const root = () => container.querySelector('[data-record-view]') as HTMLElement;
    expect(root().className).toContain('w-[40%]');
    // Transcript's tab strip is the record strip height, so its rule meets the meeting bar's.
    expect(screen.getByTestId('record-rail-strip').className).toContain('min-h-11');
    for (const view of ['notes', 'agenda'] as const) {
      rerender(<RecordRail meetingId="m-1" view={view} />);
      expect(root().className).toContain('flex-1');
      expect(root().className).not.toContain('w-[40%]');
    }
    await screen.findByRole('textbox', { hidden: true });
  });

  it('My Notes / Agenda: no sub-tabs, no label, the panel the view names', async () => {
    const { rerender } = render(<RecordRail meetingId="m-1" view="notes" />);
    expect(screen.queryByRole('tablist')).toBeNull();
    expect(screen.getByTestId('record-rail-strip')).toHaveTextContent('');
    expect(await screen.findByRole('textbox')).toBeVisible();
    expect(screen.queryByTestId('prep-panel')).toBeNull();

    rerender(<RecordRail meetingId="m-1" view="agenda" />);
    expect(screen.getByTestId('record-rail-strip')).toBeEmptyDOMElement();
    expect(screen.getByTestId('prep-panel')).toBeInTheDocument();
    expect(screen.getByTestId('prep-panel').closest('#record-tabpanel-prep')).not.toHaveAttribute('hidden');
    expect(screen.getByRole('textbox', { hidden: true }).closest('#record-tabpanel-notes')).toHaveAttribute('hidden');
    // No tabpanel roles without a tab strip to label them.
    expect(screen.queryAllByRole('tabpanel')).toHaveLength(0);
  });

  it('one PrepPanel serves the rail\'s Agenda tab and the Agenda view, mounted lazily once', async () => {
    const user = userEvent.setup();
    const { rerender } = render(<RecordRail meetingId="m-1" view="transcript" />);
    expect(screen.queryByTestId('prep-panel')).toBeNull();
    await user.click(screen.getByRole('tab', { name: 'Agenda' }));
    expect(screen.getAllByTestId('prep-panel')).toHaveLength(1);
    rerender(<RecordRail meetingId="m-1" view="agenda" />);
    rerender(<RecordRail meetingId="m-1" view="notes" />);
    rerender(<RecordRail meetingId="m-1" view="transcript" />);
    expect(screen.getAllByTestId('prep-panel', { exact: true })).toHaveLength(1);
    expect(prepMounts.count).toBe(1);
  });

  it('Agenda view before a meeting row exists shows My Notes', async () => {
    render(<RecordRail meetingId={null} view="agenda" />);
    expect(screen.queryByTestId('prep-panel')).toBeNull();
    expect(await screen.findByRole('textbox')).toBeVisible();
  });

  it('Transcript: the rail starts with its tab strip, To-Dos included; My Notes: no To-Dos', async () => {
    const user = userEvent.setup();
    const { rerender, container } = render(<RecordRail meetingId="m-1" view="transcript" />);
    expect(screen.getAllByRole('tab').map((t) => t.textContent)).toEqual(['My Notes', 'Agenda', 'To-Dos']);
    expect(screen.getByRole('tablist', { name: 'Recording side panel' })).toBeInTheDocument();
    // No Topics strip above it any more: the tab strip is the rail's first child.
    expect((container.querySelector('[data-record-view]') as HTMLElement).firstElementChild).toBe(
      screen.getByTestId('record-rail-strip'),
    );

    await user.click(screen.getByRole('tab', { name: 'To-Dos' }));
    expect(screen.getByTestId('record-todos')).toHaveAttribute('data-heading', 'false');
    expect(screen.getByTestId('record-todos').closest('[role="tabpanel"]')).not.toHaveAttribute('hidden');
    // The notepad stays mounted behind the To-Dos tab.
    const editor = await screen.findByRole('textbox', { hidden: true });

    // My Notes: no To-Dos (they're in the page's sidebar), the notepad shown.
    rerender(<RecordRail meetingId="m-1" view="notes" />);
    expect(screen.queryByTestId('record-todos')).toBeNull();
    expect(screen.getByRole('textbox')).toBe(editor);

    // Back to Transcript: the To-Dos tab is where it was left.
    rerender(<RecordRail meetingId="m-1" view="transcript" />);
    expect(screen.getByRole('tab', { name: 'To-Dos' })).toHaveAttribute('aria-selected', 'true');
    expect(screen.getByRole('textbox', { hidden: true })).toBe(editor);
  });

  it('My Notes / Agenda: the page\'s meeting bar heads the main column; Transcript: never', async () => {
    const bar = <div data-testid="meeting-bar">bar</div>;
    const { rerender } = render(<RecordRail meetingId="m-1" view="notes" topBar={bar} />);
    expect(screen.getByTestId('record-rail-strip')).toContainElement(screen.getByTestId('meeting-bar'));
    rerender(<RecordRail meetingId="m-1" view="transcript" topBar={bar} />);
    expect(screen.queryByTestId('meeting-bar')).toBeNull();
    await screen.findByRole('textbox');
  });

  it('no To-Dos tab before the meeting row exists', async () => {
    render(<RecordRail meetingId={null} view="transcript" />);
    expect(screen.queryByRole('tab', { name: 'To-Dos' })).toBeNull();
    await screen.findByRole('textbox');
  });
});
