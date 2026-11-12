import { describe, it, expect, vi } from 'vitest';
import { render, screen, within, fireEvent, waitFor } from '@testing-library/react';

vi.mock('@/components/CommandPalette/SearchMeetingsButton', () => ({
  SearchMeetingsButton: vi.fn(() => null),
}));
vi.mock('@/components/DeferredBacklog/ProcessMeetingsButton', () => ({
  ProcessMeetingsButton: vi.fn(() => null),
}));
const openImportDialog = vi.fn();
vi.mock('@/contexts/ImportDialogContext', () => ({ useImportDialog: () => ({ openImportDialog }) }));

import { MeetingsHeader, MeetingsViewTabs } from '@/components/Meetings/MeetingsHeader';
import type { MeetingsView } from '@/lib/meetings-view';

function renderHeader(view: MeetingsView, calendarConnected: boolean) {
  const onChooseView = vi.fn();
  const onAddMeeting = vi.fn();
  render(
    <>
      <MeetingsHeader
        subtitle="Thursday"
        calendarConnected={calendarConnected}
        onAddMeeting={onAddMeeting}
      />
      <MeetingsViewTabs
        view={view}
        calendarConnected={calendarConnected}
        onChooseView={onChooseView}
      />
    </>,
  );
  const labels = () =>
    within(screen.getByRole('tablist', { name: 'Meeting view' }))
      .getAllByRole('tab')
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
    expect(screen.getByRole('tab', { name: 'Agenda' })).toHaveAttribute('aria-selected', 'true');
  });

  // Design-system icon policy (docs/design-system.md §Icons, owner decision 2026-11-11):
  // view tabs are text-only — the per-tab calendar/list glyphs were decorative.
  it('draws the view options as text, with no per-tab icons', () => {
    renderHeader('agenda', true);
    for (const name of ['Agenda', 'Week', 'Month', 'List']) {
      expect(screen.getByRole('tab', { name }).querySelector('svg')).toBeNull();
    }
  });

  it('offers only Month | List without one', () => {
    const { labels } = renderHeader('list', false);
    expect(labels()).toEqual(['Month', 'List']);
  });

  // Radix tabs select on mousedown (and on focus via the arrow keys), not on click.
  it('reports the chosen view', () => {
    const { onChooseView } = renderHeader('agenda', true);
    fireEvent.mouseDown(screen.getByRole('tab', { name: 'Month' }), { button: 0 });
    expect(onChooseView).toHaveBeenCalledWith('month');
  });

  // Tabs switch the view (docs/design-system.md): a tablist, one Tab stop, arrow keys move.
  it('is a tablist with roving focus that selects on arrow keys', async () => {
    const { onChooseView } = renderHeader('agenda', true);
    const agenda = screen.getByRole('tab', { name: 'Agenda' });
    agenda.focus();
    fireEvent.keyDown(agenda, { key: 'ArrowRight' });
    // Radix moves focus on the next tick; focusing a tab selects it (automatic activation).
    await waitFor(() => expect(document.activeElement).toBe(screen.getByRole('tab', { name: 'Week' })));
    expect(onChooseView).toHaveBeenCalledWith('week');
  });

  it('does not point aria-controls at panels that are not rendered', () => {
    renderHeader('agenda', true);
    expect(screen.getByRole('tab', { name: 'Agenda' })).not.toHaveAttribute('aria-controls');
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

  it.each([
    ['agenda', true],
    ['week', true],
    ['month', true],
    ['list', true],
    ['month', false],
    ['list', false],
  ] as const)('offers Import audio as an icon button on %s (calendar: %s)', (view, calendar) => {
    renderHeader(view, calendar);
    const button = screen.getByRole('button', { name: /import audio/i });
    // Icon-only, like Search: the label lives in aria-label/title, not visible text.
    expect(button.textContent).toBe('');
    fireEvent.click(button);
    expect(openImportDialog).toHaveBeenCalled();
  });
});
