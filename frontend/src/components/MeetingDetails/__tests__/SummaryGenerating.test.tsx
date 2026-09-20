import { describe, it, expect } from 'vitest';
import { render, screen } from '@testing-library/react';
import { SummaryGenerating } from '@/components/MeetingDetails/SummaryGenerating';

// specs/0066 — the Summary tab used to show a bordered circle spinning on its axis, the
// last piece of meetily chrome in a machine that otherwise speaks entirely in tape-deck
// terms. It rewinds now: the deck going back over a tape it already has.

describe('SummaryGenerating', () => {
  it('rewinds the reels instead of spinning a borrowed circle', () => {
    const { container } = render(<SummaryGenerating />);

    expect(container.querySelector('[data-state="rewinding"]')).not.toBeNull();
    // The old spinner was a bordered div turned by `animate-spin`. Neither should return.
    expect(container.querySelector('.animate-spin')).toBeNull();
    expect(container.innerHTML).not.toContain('border-t-2');
  });

  it('says what the machine is doing, in the deck\'s own words', () => {
    render(<SummaryGenerating />);
    expect(screen.getByText('Reading the tape')).toBeInTheDocument();
    expect(screen.getByText('Writing your summary…')).toBeInTheDocument();
  });

  it('announces itself to a screen reader, which cannot see reels turn', () => {
    render(<SummaryGenerating />);
    const status = screen.getByRole('status');
    expect(status).toHaveAttribute('aria-live', 'polite');
    expect(status).toHaveAttribute('aria-label', 'Writing your summary');
  });

  // The backend attaches chunk accounting to the stored result and emits nothing while the
  // run is in flight, so any "pass 2 of 5" here would be invented. If that ever changes,
  // this test should be the thing that fails.
  it('claims no progress it cannot measure', () => {
    const { container } = render(<SummaryGenerating />);
    expect(container.textContent).not.toMatch(/\d+\s*(of|\/)\s*\d+/i);
    expect(container.textContent).not.toMatch(/%/);
  });
});
