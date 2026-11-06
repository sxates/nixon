import { describe, it, expect, vi } from 'vitest';
import { render, screen } from '@testing-library/react';

// The Agenda / Week toolbar on the Meetings page. The view switch moved up into the
// Meetings header (one switch for all four views), and the "All meetings" link went with
// the page it pointed at — this pins both out of the toolbar.

vi.mock('next/navigation', () => ({ useRouter: () => ({ push: vi.fn() }) }));
vi.mock('sonner', () => ({ toast: { success: vi.fn(), error: vi.fn(), info: vi.fn() } }));
vi.mock('@/hooks/useActionItemMutations', () => ({
  useActionItemMutations: () => ({ add: vi.fn() }),
}));

import { TodayToolbar } from '@/components/Today/TodayToolbar';

function renderToolbar(viewMode: 'day' | 'week' = 'day', viewIsToday = true) {
  render(
    <TodayToolbar
      viewMode={viewMode}
      viewDate="2026-09-13"
      navLabel="Sunday, September 13"
      viewIsToday={viewIsToday}
      onPrev={vi.fn()}
      onNext={vi.fn()}
      onToday={vi.fn()}
      onGoToDate={vi.fn()}
    />,
  );
}

describe('TodayToolbar', () => {
  it('has no view switch and no All meetings link of its own', () => {
    renderToolbar('day');
    expect(screen.queryByRole('group', { name: 'Agenda view' })).toBeNull();
    expect(screen.queryByRole('button', { name: /all meetings/i })).toBeNull();
  });

  it('names its arrows for the mode in view', () => {
    renderToolbar('week');
    expect(screen.getByRole('button', { name: 'Previous week' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Next week' })).toBeInTheDocument();
  });

  it('offers Today only when the day in view is not today', () => {
    renderToolbar('day', true);
    expect(screen.queryByRole('button', { name: 'Today' })).toBeNull();
  });

  it('keeps Add to-do', () => {
    renderToolbar('day');
    expect(screen.getByRole('button', { name: /add to-do/i })).toBeInTheDocument();
  });
});
