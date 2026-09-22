import { describe, it, expect, vi } from 'vitest';
import { render, screen, fireEvent, within } from '@testing-library/react';

// specs/0057 Task 2 — the view toggle moved from a hand-rolled pill pair to the shared
// SegmentedControl. `aria-pressed` (and the "Agenda view" group label) is the contract
// assistive tech reads, so it is what this pins down.
//
// Owner feedback 2026-09-21 renamed the "Day" LABEL to "Agenda" and reordered the group to
// Agenda | List | Week. The stored VALUES are deliberately unchanged — `'day'` is persisted
// under `nixon.today.viewMode` and appears in `?view=` — so `onSwitchMode` still speaks the
// old vocabulary.

vi.mock('next/navigation', () => ({ useRouter: () => ({ push: vi.fn() }) }));
vi.mock('sonner', () => ({ toast: { success: vi.fn(), error: vi.fn(), info: vi.fn() } }));
vi.mock('@/hooks/useActionItemMutations', () => ({
  useActionItemMutations: () => ({ add: vi.fn() }),
}));

import { TodayToolbar } from '@/components/Today/TodayToolbar';

function renderToolbar(viewMode: 'day' | 'week' = 'day', onSwitchMode = vi.fn()) {
  render(
    <TodayToolbar
      viewMode={viewMode}
      viewDate="2026-09-13"
      navLabel="Sunday, September 13"
      viewIsToday
      onPrev={vi.fn()}
      onNext={vi.fn()}
      onToday={vi.fn()}
      onGoToDate={vi.fn()}
      onSwitchMode={onSwitchMode}
    />
  );
  return { onSwitchMode };
}

describe('TodayToolbar view toggle', () => {
  it('renders the modes as an "Agenda view" group with aria-pressed on the active mode', () => {
    renderToolbar('day');
    const group = screen.getByRole('group', { name: 'Agenda view' });
    expect(within(group).getByRole('button', { name: 'Agenda' }).getAttribute('aria-pressed')).toBe('true');
    expect(within(group).getByRole('button', { name: 'Week' }).getAttribute('aria-pressed')).toBe('false');
    // The old label is gone, not merely relabelled somewhere else.
    expect(within(group).queryByRole('button', { name: 'Day' })).toBeNull();
  });

  it('reads Agenda | List | Week, left to right', () => {
    renderToolbar('day');
    const group = screen.getByRole('group', { name: 'Agenda view' });
    expect(
      within(group)
        .getAllByRole('button')
        .map((b) => b.textContent?.trim()),
    ).toEqual(['Agenda', 'List', 'Week']);
  });

  it('moves aria-pressed when the week view is active', () => {
    renderToolbar('week');
    const group = screen.getByRole('group', { name: 'Agenda view' });
    expect(within(group).getByRole('button', { name: 'Week' }).getAttribute('aria-pressed')).toBe('true');
    expect(within(group).getByRole('button', { name: 'Agenda' }).getAttribute('aria-pressed')).toBe('false');
  });

  // The rename must not have leaked into the value the toolbar reports upward.
  it('still reports the persisted value, not the new label, for the agenda mode', () => {
    const { onSwitchMode } = renderToolbar('week');
    fireEvent.click(screen.getByRole('button', { name: 'Agenda' }));
    expect(onSwitchMode).toHaveBeenCalledWith('day');
  });

  it('calls onSwitchMode with the chosen mode', () => {
    const { onSwitchMode } = renderToolbar('day');
    fireEvent.click(screen.getByRole('button', { name: 'Week' }));
    expect(onSwitchMode).toHaveBeenCalledWith('week');
  });
});
