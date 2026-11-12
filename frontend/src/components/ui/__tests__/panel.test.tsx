import { describe, it, expect } from 'vitest';
import { render, screen } from '@testing-library/react';

import { Panel } from '@/components/ui/panel';

describe('Panel', () => {
  it('is the card: rounded-lg border bg-card shadow-card, px-4 py-3', () => {
    const { container } = render(<Panel>body</Panel>);
    const root = container.firstElementChild!;
    expect(root.tagName).toBe('DIV');
    expect(root).toHaveClass('rounded-lg', 'border', 'bg-card', 'shadow-card', 'px-4', 'py-3');
    expect(root).toHaveAttribute('data-variant', 'solid');
  });

  it('document size pads p-6', () => {
    const { container } = render(<Panel size="document">doc</Panel>);
    const root = container.firstElementChild!;
    expect(root).toHaveClass('p-6');
    expect(root).not.toHaveClass('px-4');
  });

  it('dashed is a flat placeholder: dashed hairline, no shadow', () => {
    const { container } = render(<Panel variant="dashed">empty</Panel>);
    const root = container.firstElementChild!;
    expect(root).toHaveClass('border-dashed');
    expect(root).not.toHaveClass('shadow-card');
  });

  it('a title makes it a section named by an engraved heading', () => {
    render(
      <Panel title="Speakers" count={4}>
        chips
      </Panel>,
    );
    const heading = screen.getByRole('heading', { name: 'Speakers' });
    expect(heading.tagName).toBe('H2');
    expect(heading).toHaveClass('u-section-label');
    expect(screen.getByRole('region', { name: 'Speakers' })).toHaveAttribute('aria-labelledby', heading.id);
    expect(screen.getByText('4')).toHaveAttribute('data-panel-count');
  });

  it('shows a zero count but not a null one', () => {
    const { rerender, container } = render(<Panel title="Action items" count={0} />);
    expect(container.querySelector('[data-panel-count]')?.textContent).toBe('0');
    rerender(<Panel title="Action items" count={null} />);
    expect(container.querySelector('[data-panel-count]')).toBeNull();
  });

  it('puts the action on the right of the header', () => {
    render(<Panel title="Participants" action={<button type="button">Add</button>} />);
    const action = screen.getByRole('button', { name: 'Add' }).parentElement!;
    expect(action).toHaveClass('ml-auto');
  });

  it('takes a heading level', () => {
    render(<Panel title="Brief" titleAs="h3" />);
    expect(screen.getByRole('heading', { name: 'Brief' }).tagName).toBe('H3');
  });
});
