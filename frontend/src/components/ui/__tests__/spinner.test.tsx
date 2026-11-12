import { describe, it, expect } from 'vitest';
import { render, screen } from '@testing-library/react';

import { Spinner } from '@/components/ui/spinner';

describe('Spinner', () => {
  it('is a 14px spinning glyph by default, hidden from assistive tech', () => {
    const { container } = render(<Spinner />);
    const svg = container.querySelector('svg')!;
    expect(svg.getAttribute('width')).toBe('14');
    expect(svg.getAttribute('class')).toContain('animate-spin');
    expect(svg.getAttribute('aria-hidden')).toBe('true');
    expect(container.querySelector('[role="status"]')).toBeNull();
  });

  it('has a 20px md size', () => {
    const { container } = render(<Spinner size="md" />);
    expect(container.querySelector('svg')!.getAttribute('width')).toBe('20');
  });

  it('inherits colour (no text-* of its own)', () => {
    const { container } = render(<Spinner />);
    expect(container.querySelector('svg')!.getAttribute('class')).not.toMatch(/\btext-/);
  });

  it('becomes a labelled status when given a label', () => {
    render(<Spinner label="Loading meetings" />);
    expect(screen.getByRole('status', { name: 'Loading meetings' })).toBeTruthy();
  });
});
