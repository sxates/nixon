import { describe, it, expect, vi } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';

import { DateNavigator } from '@/components/ui/date-navigator';

describe('DateNavigator', () => {
  it('prev / label / next with named arrows, label is a live region', () => {
    const onPrev = vi.fn();
    const onNext = vi.fn();
    render(
      <DateNavigator label="November 2026" onPrev={onPrev} onNext={onNext} prevLabel="Previous month" nextLabel="Next month" />,
    );
    fireEvent.click(screen.getByRole('button', { name: 'Previous month' }));
    fireEvent.click(screen.getByRole('button', { name: 'Next month' }));
    expect(onPrev).toHaveBeenCalledOnce();
    expect(onNext).toHaveBeenCalledOnce();
    expect(screen.getByText('November 2026')).toHaveAttribute('aria-live', 'polite');
    expect(screen.queryByRole('button', { name: 'Today' })).toBeNull();
  });

  it('Today sits left of the arrows and can be hidden', () => {
    const onToday = vi.fn();
    const { rerender } = render(
      <DateNavigator label="Tue, Nov 11" onPrev={() => {}} onNext={() => {}} onToday={onToday} />,
    );
    const buttons = screen.getAllByRole('button');
    expect(buttons[0]).toHaveTextContent('Today');
    expect(buttons[0]).toHaveClass('h-7');
    fireEvent.click(buttons[0]!);
    expect(onToday).toHaveBeenCalledOnce();
    rerender(<DateNavigator label="Tue, Nov 11" onPrev={() => {}} onNext={() => {}} onToday={onToday} showToday={false} />);
    expect(screen.queryByRole('button', { name: 'Today' })).toBeNull();
  });

  it('places leading and trailing slots', () => {
    render(
      <DateNavigator
        label="Nov 10 – 16"
        onPrev={() => {}}
        onNext={() => {}}
        leading={<span data-testid="spin" />}
        trailing={<span data-testid="picker" />}
      />,
    );
    const group = screen.getByRole('group', { name: 'Date navigation' });
    const order = Array.from(group.querySelectorAll('[data-testid], [aria-live], button')).map(
      (el) => el.getAttribute('data-testid') ?? el.getAttribute('aria-label') ?? 'label',
    );
    expect(order).toEqual(['spin', 'Previous', 'label', 'picker', 'Next']);
  });
});
