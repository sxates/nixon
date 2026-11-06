import { describe, it, expect, vi } from 'vitest';
import { render, screen, within, fireEvent } from '@testing-library/react';

vi.mock('@/components/CommandPalette/SearchMeetingsButton', () => ({
  SearchMeetingsButton: vi.fn(() => null),
}));
vi.mock('@/components/DeferredBacklog/ProcessMeetingsButton', () => ({
  ProcessMeetingsButton: vi.fn(() => null),
}));
const openImportDialog = vi.fn();
vi.mock('@/contexts/ImportDialogContext', () => ({ useImportDialog: () => ({ openImportDialog }) }));

import { MeetingsHeader } from '@/components/Meetings/MeetingsHeader';
import type { MeetingsView } from '@/lib/meetings-view';

function renderHeader(view: MeetingsView, calendarConnected: boolean) {
  const onChooseView = vi.fn();
  const onAddMeeting = vi.fn();
  render(
    <MeetingsHeader
      subtitle="Thursday"
      view={view}
      calendarConnected={calendarConnected}
      onChooseView={onChooseView}
      onAddMeeting={onAddMeeting}
    />,
  );
  const labels = () =>
    within(screen.getByRole('group', { name: 'Meeting view' }))
      .getAllByRole('button')
      .map((b) => b.textContent?.trim());
  return { onChooseView, onAddMeeting, labels };
}

describe('MeetingsHeader', () => {
  it('is titled Meetings', () => {
    renderHeader('agenda', true);
    expect(screen.getByRole('heading', { name: 'Meetings' })).toBeInTheDocument();
  });

  it('offers Agenda | Week | Month | List with a calendar connected', () => {
    const { labels } = renderHeader('agenda', true);
    expect(labels()).toEqual(['Agenda', 'Week', 'Month', 'List']);
    expect(screen.getByRole('button', { name: 'Agenda' })).toHaveAttribute('aria-pressed', 'true');
  });

  it('gives every view option the same-size icon', () => {
    renderHeader('agenda', true);
    for (const name of ['Agenda', 'Week', 'Month', 'List']) {
      const icon = screen.getByRole('button', { name }).querySelector('svg');
      expect(icon, `${name} has no icon`).not.toBeNull();
      expect(icon!.getAttribute('class')).toContain('h-3.5 w-3.5');
    }
  });

  it('offers only Month | List without one', () => {
    const { labels } = renderHeader('list', false);
    expect(labels()).toEqual(['Month', 'List']);
  });

  it('reports the chosen view', () => {
    const { onChooseView } = renderHeader('agenda', true);
    fireEvent.click(screen.getByRole('button', { name: 'Month' }));
    expect(onChooseView).toHaveBeenCalledWith('month');
  });

  // specs/0069 W3 — one primary Add meeting; Record and New note stay gone.
  it('has one Add meeting action with a calendar, and no Record or New note', () => {
    const { onAddMeeting } = renderHeader('agenda', true);
    fireEvent.click(screen.getByRole('button', { name: /add meeting/i }));
    expect(onAddMeeting).toHaveBeenCalledTimes(1);
    expect(screen.queryByRole('button', { name: /^record$/i })).toBeNull();
    expect(screen.queryByRole('button', { name: /new note/i })).toBeNull();
  });

  it.each([
    ['agenda', true],
    ['week', true],
    ['month', true],
    ['list', true],
    ['month', false],
    ['list', false],
  ] as const)('carries no All meetings or to-do link (%s, calendar %s)', (view, connected) => {
    renderHeader(view, connected);
    expect(screen.queryByRole('button', { name: /all meetings/i })).toBeNull();
    expect(screen.queryByRole('button', { name: /to-do/i })).toBeNull();
  });

  it('offers Import audio on List and Month, where an import lands', () => {
    renderHeader('list', false);
    fireEvent.click(screen.getByRole('button', { name: /import audio/i }));
    expect(openImportDialog).toHaveBeenCalled();
  });
});
