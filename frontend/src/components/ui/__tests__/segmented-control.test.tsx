import { useState } from 'react';
import { describe, it, expect, vi } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';

import { SegmentedControl } from '@/components/ui/segmented-control';

const OPTIONS = [
  { value: 'day', label: 'Day' },
  { value: 'week', label: 'Week' },
  { value: 'month', label: 'Month' },
];

function renderControl(value = 'day', onChange = vi.fn()) {
  const utils = render(
    <SegmentedControl options={OPTIONS} value={value} onChange={onChange} aria-label="Agenda view" />
  );
  return { ...utils, onChange };
}

describe('SegmentedControl', () => {
  it('exposes a labelled group of pressable options', () => {
    renderControl();
    const group = screen.getByRole('group', { name: 'Agenda view' });
    expect(group).toBeTruthy();
    expect(screen.getAllByRole('button')).toHaveLength(3);
  });

  it('marks only the active option aria-pressed', () => {
    renderControl('week');
    expect(screen.getByRole('button', { name: 'Day' }).getAttribute('aria-pressed')).toBe('false');
    expect(screen.getByRole('button', { name: 'Week' }).getAttribute('aria-pressed')).toBe('true');
  });

  it('underlines the active option in brand', () => {
    renderControl('week');
    expect(screen.getByRole('button', { name: 'Week' }).className).toContain('border-brand');
    expect(screen.getByRole('button', { name: 'Day' }).className).toContain('border-transparent');
  });

  it('fires onChange when an option is clicked', () => {
    const { onChange } = renderControl('day');
    fireEvent.click(screen.getByRole('button', { name: 'Month' }));
    expect(onChange).toHaveBeenCalledWith('month');
  });

  it('does not fire onChange when the active option is clicked', () => {
    const { onChange } = renderControl('day');
    fireEvent.click(screen.getByRole('button', { name: 'Day' }));
    expect(onChange).not.toHaveBeenCalled();
  });

  it('uses a roving tabIndex so the group is one Tab stop', () => {
    renderControl('week');
    expect(screen.getByRole('button', { name: 'Week' }).tabIndex).toBe(0);
    expect(screen.getByRole('button', { name: 'Day' }).tabIndex).toBe(-1);
  });

  // The roving tabIndex means arrow keys MUST move DOM focus as well as the
  // selection: selecting without focusing strands the user on a button that just
  // became tabIndex={-1}, with the next arrow press landing on the wrong element
  // (or doing nothing, since the old button's index is now the active one). These
  // run against a stateful wrapper — a static `value` prop would hide exactly that.
  describe('keyboard navigation (stateful)', () => {
    function StatefulControl({ initial = 'day' }: { initial?: string }) {
      const [value, setValue] = useState(initial);
      return (
        <SegmentedControl
          options={OPTIONS}
          value={value}
          onChange={setValue}
          aria-label="Agenda view"
        />
      );
    }

    const press = (name: string, key: string) =>
      fireEvent.keyDown(screen.getByRole('button', { name }), { key });

    it('ArrowRight selects the next option AND moves focus to it', () => {
      render(<StatefulControl />);
      press('Day', 'ArrowRight');

      const week = screen.getByRole('button', { name: 'Week' });
      expect(week.getAttribute('aria-pressed')).toBe('true');
      expect(document.activeElement).toBe(week);
      expect(week.tabIndex).toBe(0);
      expect(screen.getByRole('button', { name: 'Day' }).tabIndex).toBe(-1);
    });

    it('keeps advancing on repeated ArrowRight and wraps to the first option', () => {
      render(<StatefulControl />);
      press('Day', 'ArrowRight');
      press('Week', 'ArrowRight');
      expect(screen.getByRole('button', { name: 'Month' }).getAttribute('aria-pressed')).toBe('true');
      expect(document.activeElement).toBe(screen.getByRole('button', { name: 'Month' }));

      press('Month', 'ArrowRight');
      const day = screen.getByRole('button', { name: 'Day' });
      expect(day.getAttribute('aria-pressed')).toBe('true');
      expect(document.activeElement).toBe(day);
    });

    it('ArrowLeft wraps backwards to the last option and focuses it', () => {
      render(<StatefulControl />);
      press('Day', 'ArrowLeft');
      const month = screen.getByRole('button', { name: 'Month' });
      expect(month.getAttribute('aria-pressed')).toBe('true');
      expect(document.activeElement).toBe(month);

      press('Month', 'ArrowLeft');
      const week = screen.getByRole('button', { name: 'Week' });
      expect(week.getAttribute('aria-pressed')).toBe('true');
      expect(document.activeElement).toBe(week);
    });

    it('Home jumps to the first option and End to the last', () => {
      render(<StatefulControl initial="week" />);
      press('Week', 'End');
      expect(screen.getByRole('button', { name: 'Month' }).getAttribute('aria-pressed')).toBe('true');
      expect(document.activeElement).toBe(screen.getByRole('button', { name: 'Month' }));

      press('Month', 'Home');
      expect(screen.getByRole('button', { name: 'Day' }).getAttribute('aria-pressed')).toBe('true');
      expect(document.activeElement).toBe(screen.getByRole('button', { name: 'Day' }));
    });

    it('leaves exactly one option in the Tab order after arrow navigation', () => {
      render(<StatefulControl />);
      press('Day', 'ArrowRight');
      const tabbable = screen.getAllByRole('button').filter((b) => b.tabIndex === 0);
      expect(tabbable).toHaveLength(1);
      expect(tabbable[0].textContent).toBe('Week');
    });
  });

  it('renders an option icon when one is given', () => {
    render(
      <SegmentedControl
        options={[{ value: 'list', label: 'List', icon: <svg data-testid="list-icon" /> }]}
        value="list"
        onChange={vi.fn()}
        aria-label="View mode"
      />
    );
    expect(screen.getByTestId('list-icon')).toBeTruthy();
  });
});
