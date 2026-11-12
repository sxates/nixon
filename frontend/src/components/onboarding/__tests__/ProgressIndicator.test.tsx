import { describe, it, expect } from 'vitest';
import { render, screen } from '@testing-library/react';
import { ProgressIndicator } from '../shared/ProgressIndicator';

// specs/0057 Task 7 — the onboarding progress indicator is a row of engraved
// position-marker bars: done (bg-success), active (bg-brand), pending (bg-border).
// Design-system wave C dropped the per-step icons and the active glow; this test
// asserts on the bar's class list per state.

function markerBar(step: number, total: number): Element {
  const button = screen.getByRole('button', { name: new RegExp(`Step ${step} of ${total}`) });
  const bar = button.querySelector('span');
  if (!bar) throw new Error(`No marker bar found for step ${step}`);
  return bar;
}

describe('ProgressIndicator', () => {
  it('renders a done bar (bg-success), an active bar (bg-brand), and a pending bar (bg-border)', () => {
    render(<ProgressIndicator current={2} total={4} />);

    // Step 1 is done (before current).
    expect(markerBar(1, 4).className).toContain('bg-success');

    // Step 2 is the active/current step.
    const activeBar = markerBar(2, 4);
    expect(activeBar.className).toContain('bg-brand');
    expect(activeBar.className).not.toContain('shadow-[');

    // Step 4 is pending (after current).
    expect(markerBar(4, 4).className).toContain('bg-border');
  });

  it('draws no per-step icon above the bars', () => {
    render(<ProgressIndicator current={2} total={4} />);
    for (const step of [1, 2, 3, 4]) {
      const button = screen.getByRole('button', { name: new RegExp(`Step ${step} of 4`) });
      expect(button.querySelector('svg')).toBeNull();
    }
  });

  it('uses engraved bar dimensions on every marker', () => {
    render(<ProgressIndicator current={1} total={4} />);
    for (const step of [1, 2, 3, 4]) {
      const bar = markerBar(step, 4);
      expect(bar.className).toContain('h-2');
      expect(bar.className).toContain('w-6');
      expect(bar.className).toContain('rounded-sm');
    }
  });
});
