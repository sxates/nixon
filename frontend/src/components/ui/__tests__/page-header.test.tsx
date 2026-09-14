import { describe, it, expect } from 'vitest';
import { render, screen } from '@testing-library/react';

import { PageHeader } from '@/components/ui/page-header';

// specs/0057 Task 2 — the one page-chrome header. Every screen's title block goes
// through this, so the contract is: an <h1> with the title, an optional .u-meta
// subtitle, and an optional actions cluster on the right.
describe('PageHeader', () => {
  it('renders the title as the page h1', () => {
    render(<PageHeader title="All meetings" />);
    const heading = screen.getByRole('heading', { level: 1, name: 'All meetings' });
    expect(heading).toBeTruthy();
    expect(heading.className).toContain('font-display');
  });

  it('renders a ReactNode title (person details passes an inline editor)', () => {
    render(<PageHeader title={<span data-testid="custom-title">Ada Lovelace</span>} />);
    expect(screen.getByTestId('custom-title')).toBeTruthy();
    expect(screen.getByRole('heading', { level: 1 }).textContent).toBe('Ada Lovelace');
  });

  it('renders the subtitle in a .u-meta paragraph', () => {
    const { container } = render(<PageHeader title="People" subtitle="3 people" />);
    const meta = container.querySelector('p.u-meta');
    expect(meta).toBeTruthy();
    expect(meta!.textContent).toBe('3 people');
  });

  it('omits the subtitle paragraph when none is given', () => {
    const { container } = render(<PageHeader title="Ask AI" />);
    expect(container.querySelector('p.u-meta')).toBeNull();
  });

  it('renders actions in a right-hand cluster', () => {
    const { container } = render(
      <PageHeader title="People" actions={<button type="button">Add person</button>} />
    );
    expect(screen.getByRole('button', { name: 'Add person' })).toBeTruthy();
    const cluster = container.querySelector('header > div:last-child');
    expect(cluster!.className).toContain('items-center');
  });

  it('renders a banner landmark with the shared chrome padding', () => {
    const { container } = render(<PageHeader title="Tasks" className="pt-2" />);
    const header = container.querySelector('header')!;
    expect(header.className).toContain('px-7');
    expect(header.className).toContain('pt-2');
  });
});
