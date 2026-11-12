import { describe, it, expect, vi } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';

import { PageHeader } from '@/components/ui/page-header';

// specs/0057 Task 2 — the one page-chrome header. Every screen's title block goes
// through this, so the contract is: an <h1> with the title, an optional .u-meta
// subtitle, and an optional actions cluster on the right.
describe('PageHeader', () => {
  it('renders the title as the page h1', () => {
    render(<PageHeader title="All meetings" />);
    const heading = screen.getByRole('heading', { level: 1, name: 'All meetings' });
    expect(heading).toBeTruthy();
    expect(heading.className).toContain('u-page-title');
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

  it('renders a back IconButton before the title when given', () => {
    const onClick = vi.fn();
    const { container } = render(<PageHeader title="Ada" back={{ label: 'People', onClick }} />);
    const back = screen.getByRole('button', { name: 'People' });
    fireEvent.click(back);
    expect(onClick).toHaveBeenCalledOnce();
    // Before the h1 in document order, and not inside it.
    const h1 = container.querySelector('h1')!;
    expect(h1.contains(back)).toBe(false);
    expect(back.compareDocumentPosition(h1) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  });

  it('renders no back button by default', () => {
    render(<PageHeader title="People" />);
    expect(screen.queryByRole('button')).toBeNull();
  });

  it('renders a node subtitle OUTSIDE the h1 so an editor keeps its own weight', () => {
    const { container } = render(
      <PageHeader title="Ada" subtitle={<input aria-label="Person title" defaultValue="CTO" />} />
    );
    const editor = screen.getByLabelText('Person title');
    expect(container.querySelector('h1')!.contains(editor)).toBe(false);
    // A node subtitle is not phrasing content, so it is a div, still in .u-meta.
    expect(editor.closest('div.u-meta')).toBeTruthy();
    expect(container.querySelector('p.u-meta')).toBeNull();
  });
});
