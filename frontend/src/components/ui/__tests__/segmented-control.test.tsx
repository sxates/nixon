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

  // Tabs switch the view; a boxed SegmentedControl filters it — never two underline strips
  // in a row (docs/design-system.md). The active option is a raised card cell in a recessed
  // track, not a brand underline.
  it('draws a boxed key group: recessed track, raised active cell, no underline', () => {
    renderControl('week');
    const group = screen.getByRole('group', { name: 'Agenda view' });
    expect(group.className).toContain('bg-muted');
    expect(group.className).toContain('rounded-md');
    expect(group.className).toContain('h-7');
    const active = screen.getByRole('button', { name: 'Week' }).className;
    const idle = screen.getByRole('button', { name: 'Day' }).className;
    expect(active).toContain('bg-card');
    expect(idle).not.toContain('bg-card');
    expect(active).not.toContain('border-b-2');
    expect(active).not.toContain('border-brand');
  });

  it('is 24px in the compact size', () => {
    render(
      <SegmentedControl options={OPTIONS} value="day" onChange={vi.fn()} aria-label="Scope" size="compact" />,
    );
    expect(screen.getByRole('group', { name: 'Scope' }).className).toContain('h-6');
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

  // A value that matches no option (Action items' "Mine | Everyone" while a person filter
  // is on) left every option at tabIndex -1, so the group dropped out of the Tab order.
  it('keeps the first option tabbable when no option is active', () => {
    renderControl('none-of-these');
    const tabbable = screen.getAllByRole('button').filter((b) => b.tabIndex === 0);
    expect(tabbable.map((b) => b.textContent)).toEqual(['Day']);
    expect(screen.getAllByRole('button').every((b) => b.getAttribute('aria-pressed') === 'false')).toBe(true);
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

  it('defaults to the page-level size', () => {
    renderControl();
    const btn = screen.getByRole('button', { name: 'Day' });
    expect(btn.className).toContain('text-xs');
    expect(btn.className).toContain('px-3');
    expect(screen.getByRole('group').getAttribute('data-size')).toBe('default');
  });

  it('has a compact size for panels and toolbars', () => {
    render(
      <SegmentedControl options={OPTIONS} value="day" onChange={vi.fn()} aria-label="Agenda view" size="compact" />
    );
    const btn = screen.getByRole('button', { name: 'Day' });
    expect(btn.className).toContain('text-caption');
    expect(btn.className).toContain('px-2');
    expect(btn.className).not.toContain('text-sm');
    // Still the one boxed style.
    expect(btn.className).toContain('bg-card');
  });
});
