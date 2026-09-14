import { describe, it, expect } from 'vitest';
import { render, screen } from '@testing-library/react';
import { TapeCounter } from '@/components/Transport/TapeCounter';

describe('TapeCounter', () => {
  it('renders every digit in its own well and exposes the time as text', () => {
    render(<TapeCounter seconds={252} size="sm" />);
    const el = screen.getByRole('timer');
    expect(el.textContent?.replace(/\s/g, '')).toBe('00:04:12');
    expect(el.querySelectorAll('[data-digit]').length).toBe(6);
  });
  it('applies the amber tone when frozen on hold', () => {
    render(<TapeCounter seconds={1} size="sm" tone="amber" />);
    expect(screen.getByRole('timer').dataset.tone).toBe('amber');
  });

  // fix round 1 (specs/0057 Task 6, Important 2): a `decorative` counter is one of
  // many in a list — it must not claim `role="timer"` (that would make every row's
  // accessible name read "Elapsed time 00:04:12"). It still draws the digit wells.
  it('decorative yields no role="timer", no aria-label, and is aria-hidden', () => {
    const { container } = render(<TapeCounter seconds={252} size="sm" decorative />);
    expect(screen.queryByRole('timer')).not.toBeInTheDocument();
    const root = container.firstElementChild as HTMLElement;
    expect(root.getAttribute('aria-hidden')).toBe('true');
    expect(root.hasAttribute('aria-label')).toBe(false);
    expect(root.querySelectorAll('[data-digit]').length).toBe(6);
  });
});
