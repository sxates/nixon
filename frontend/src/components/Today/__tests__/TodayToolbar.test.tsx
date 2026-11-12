import { describe, it, expect, vi } from 'vitest';
import { render, screen } from '@testing-library/react';

// The Agenda / Week toolbar on the Meetings page. The view switch moved up into the
// Meetings header (one switch for all four views); the "All meetings" link went with the
// page it pointed at, and "Add to-do" left the page at the owner's request (Action items
// keeps its sidebar entry). This pins all three out of the toolbar.

import { TodayToolbar } from '@/components/Today/TodayToolbar';

function renderToolbar(viewMode: 'day' | 'week' = 'day') {
  render(
    <TodayToolbar
      viewMode={viewMode}
      viewDate="2026-09-13"
      navLabel="Sunday, September 13"
      onPrev={vi.fn()}
      onNext={vi.fn()}
      onToday={vi.fn()}
      onGoToDate={vi.fn()}
    />,
  );
}

describe('TodayToolbar', () => {
  it('has no view switch, All meetings link or Add to-do of its own', () => {
    renderToolbar('day');
    expect(screen.queryByRole('group', { name: 'Agenda view' })).toBeNull();
    expect(screen.queryByRole('button', { name: /all meetings/i })).toBeNull();
    expect(screen.queryByRole('button', { name: /to-do/i })).toBeNull();
  });

  it('names its arrows for the mode in view', () => {
    renderToolbar('week');
    expect(screen.getByRole('button', { name: 'Previous week' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Next week' })).toBeInTheDocument();
  });

  // Today is a key in every view (as in Week and Month), never only a label.
  it('offers the Today key in the day view too', () => {
    renderToolbar('day');
    expect(screen.getByRole('button', { name: 'Today' })).toBeInTheDocument();
  });
});
