import { describe, it, expect, vi } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';

import { ListRow, LinkRow } from '@/components/ui/list-row';

describe('ListRow', () => {
  it('renders title and meta with the row text roles', () => {
    render(<ListRow title="Weekly sync" meta="9:15 AM" />);
    expect(screen.getByText('Weekly sync')).toHaveClass('u-row-title');
    expect(screen.getByText('9:15 AM')).toHaveClass('u-row-meta');
    expect(screen.queryByRole('button')).toBeNull();
  });

  it('onClick makes the main area a focus-ringed button, with trailing and menu outside it', () => {
    const onClick = vi.fn();
    render(
      <ListRow
        title="Priya Patel"
        lead={<span data-testid="lead" />}
        trailing={<button type="button">Star</button>}
        menu={<button type="button">More</button>}
        onClick={onClick}
      />,
    );
    const main = screen.getByRole('button', { name: 'Priya Patel' });
    expect(main).toHaveClass('focus-visible:ring-2', 'focus-visible:ring-ring');
    expect(main).toContainElement(screen.getByTestId('lead'));
    expect(main).not.toContainElement(screen.getByRole('button', { name: 'Star' }));
    expect(main).not.toContainElement(screen.getByRole('button', { name: 'More' }));
    fireEvent.click(main);
    expect(onClick).toHaveBeenCalledOnce();
  });

  it('selected rows use the brand wash and mark the button current', () => {
    const { container } = render(<ListRow title="Row" selected onClick={() => {}} />);
    expect(container.firstElementChild).toHaveClass('bg-brand-wash');
    expect(screen.getByRole('button')).toHaveAttribute('aria-current', 'true');
  });

  it('menuReveal="hover" hides the menu until hover or focus', () => {
    const { container } = render(
      <ListRow title="Row" menu={<button type="button">More</button>} menuReveal="hover" />,
    );
    const menu = container.querySelector('[data-row-menu]')!;
    expect(menu).toHaveClass('opacity-0', 'group-hover/row:opacity-100', 'focus-within:opacity-100');
  });
});

describe('LinkRow', () => {
  it('is a button with a chevron that lights to brand on hover', () => {
    const onClick = vi.fn();
    render(<LinkRow title="Product sync" meta="Nov 3" onClick={onClick} hint="Open this meeting" />);
    const row = screen.getByRole('button', { name: /Product sync/ });
    expect(row).toHaveAttribute('title', 'Open this meeting');
    expect(row).toHaveClass('focus-visible:ring-2');
    expect(screen.getByText('Product sync')).toHaveClass('u-row-title', 'group-hover/link:text-brand');
    expect(screen.getByText('Nov 3')).toHaveClass('u-row-meta');
    expect(row.querySelector('svg')).toHaveClass('group-hover/link:text-brand');
    fireEvent.click(row);
    expect(onClick).toHaveBeenCalledOnce();
  });

  it('muted rows drop the title weight', () => {
    render(<LinkRow title="Uncited" onClick={() => {}} muted />);
    const t = screen.getByText('Uncited');
    expect(t).toHaveClass('text-muted-foreground');
    expect(t).not.toHaveClass('u-row-title');
  });
});
