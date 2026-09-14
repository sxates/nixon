import React from 'react';
import { render, screen, fireEvent } from '@testing-library/react';
import { describe, it, expect, vi, afterEach } from 'vitest';

// specs/0056 W4 — the record screen's right rail: Notes / Prep tabs. Locks in
// (a) Notes as the default, (b) PrepPanel lazily mounted on first activation and
// kept mounted afterwards, (c) the notepad staying mounted while Prep is shown
// (its debounced autosave / unmount-flush must not be disturbed by a tab switch),
// (d) the record-screen `px-5` gutter around PrepPanel (the 0054 drawer had none).

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(() => Promise.resolve(null)),
}));

vi.mock('sonner', () => ({
  toast: { error: vi.fn(), success: vi.fn() },
}));

vi.mock('@/components/Sidebar/SidebarProvider', () => ({
  useSidebar: () => ({ currentMeeting: null }),
}));

vi.mock('@/components/MeetingDetails/PrepPanel', () => ({
  PrepPanel: ({ meetingId }: { meetingId: string }) => (
    <div data-testid="prep-panel">prep for {meetingId}</div>
  ),
}));

vi.mock('@/app/_components/NotepadPanel', () => ({
  NotepadPanel: ({ meetingId }: { meetingId?: string }) => (
    <div data-testid="notepad-panel">notepad for {meetingId ?? 'none'}</div>
  ),
}));

vi.mock('@/components/Record/RecordAgendaPanel', () => ({
  RecordAgendaPanel: ({ meetingId }: { meetingId: string }) => (
    <div data-testid="agenda-panel">agenda for {meetingId}</div>
  ),
}));

vi.mock('@/hooks/usePrepAvailability', () => ({
  usePrepAvailability: () => ({ hasPrep: true, openItemCount: 3 }),
}));

import { RecordRail } from '@/components/Record/RecordRail';

afterEach(() => {
  vi.clearAllMocks();
});

describe('RecordRail (specs/0056 W4)', () => {
  it('defaults to the Notes tab and does not mount PrepPanel', async () => {
    render(<RecordRail meetingId="m-1" />);

    expect(screen.getByRole('tablist', { name: 'Recording side panel' })).toBeInTheDocument();
    expect(screen.getByRole('tab', { name: /notes/i })).toHaveAttribute('aria-selected', 'true');
    expect(screen.getByRole('tab', { name: /prep/i })).toHaveAttribute('aria-selected', 'false');

    expect(await screen.findByTestId('notepad-panel')).toBeInTheDocument();
    expect(screen.getByTestId('agenda-panel')).toBeInTheDocument();
    expect(screen.queryByTestId('prep-panel')).not.toBeInTheDocument();
  });

  it('clicking Prep mounts PrepPanel inside a px-5 gutter and keeps the notepad mounted', async () => {
    render(<RecordRail meetingId="m-1" />);
    await screen.findByTestId('notepad-panel');

    fireEvent.click(screen.getByRole('tab', { name: /prep/i }));

    const prep = screen.getByTestId('prep-panel');
    expect(prep).toBeInTheDocument();
    // The scroller directly wrapping PrepPanel carries the record-screen gutter.
    expect(prep.parentElement?.className).toContain('px-5');
    expect(screen.getByRole('tab', { name: /prep/i })).toHaveAttribute('aria-selected', 'true');

    // Notes panel stays mounted (hidden) so NotepadPanel's autosave lifecycle is untouched.
    const notepad = screen.getByTestId('notepad-panel');
    expect(notepad).toBeInTheDocument();
    expect(notepad.closest('[role="tabpanel"]')).toHaveAttribute('hidden');
    expect(prep.closest('[role="tabpanel"]')).not.toHaveAttribute('hidden');
  });

  it('switching back to Notes keeps PrepPanel mounted', async () => {
    render(<RecordRail meetingId="m-1" />);
    await screen.findByTestId('notepad-panel');

    fireEvent.click(screen.getByRole('tab', { name: /prep/i }));
    expect(screen.getByTestId('prep-panel')).toBeInTheDocument();

    fireEvent.click(screen.getByRole('tab', { name: /notes/i }));
    expect(screen.getByRole('tab', { name: /notes/i })).toHaveAttribute('aria-selected', 'true');
    const prep = screen.getByTestId('prep-panel');
    expect(prep).toBeInTheDocument();
    expect(prep.closest('[role="tabpanel"]')).toHaveAttribute('hidden');
    expect(screen.getByTestId('notepad-panel').closest('[role="tabpanel"]')).not.toHaveAttribute('hidden');
  });

  it('shows the carried-over open item count as a badge on the Prep tab', () => {
    render(<RecordRail meetingId="m-1" />);
    const prepTab = screen.getByRole('tab', { name: /prep/i });
    expect(prepTab).toHaveTextContent('3');
    expect(screen.getByLabelText('3 carried-over open items')).toBeInTheDocument();
  });

  it('renders only the Notes tab before a meeting row exists', async () => {
    render(<RecordRail meetingId={null} />);
    expect(screen.getAllByRole('tab')).toHaveLength(1);
    expect(screen.getByRole('tab', { name: /notes/i })).toBeInTheDocument();
    expect(screen.queryByRole('tab', { name: /prep/i })).not.toBeInTheDocument();
    expect(screen.queryByTestId('agenda-panel')).not.toBeInTheDocument();
    expect(await screen.findByTestId('notepad-panel')).toBeInTheDocument();
  });

  it('ArrowRight from Notes moves focus to (and activates) Prep', () => {
    render(<RecordRail meetingId="m-1" />);
    const notesTab = screen.getByRole('tab', { name: /notes/i });
    const prepTab = screen.getByRole('tab', { name: /prep/i });

    notesTab.focus();
    fireEvent.keyDown(notesTab, { key: 'ArrowRight' });

    expect(prepTab).toHaveFocus();
    expect(prepTab).toHaveAttribute('aria-selected', 'true');
    expect(prepTab).toHaveAttribute('tabindex', '0');
    expect(notesTab).toHaveAttribute('tabindex', '-1');
  });
});
