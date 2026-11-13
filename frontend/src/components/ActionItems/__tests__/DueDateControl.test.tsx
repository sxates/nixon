import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { cleanup, render, screen, fireEvent } from '@testing-library/react';
import { DueDateControl, dueDisplay } from '../DueDateControl';
import type { ActionItem } from '@/types';

// 2026-11-12 is a Thursday.
const TODAY = new Date(2026, 10, 12, 10, 0);

function item(over: Partial<ActionItem> = {}): ActionItem {
  return {
    id: 'a',
    description: 'Send deck',
    status: 'open',
    dueDate: null,
    dueHint: null,
    createdAt: new Date(2026, 10, 12, 9, 0).toISOString(),
    ...over,
  } as unknown as ActionItem;
}

beforeEach(() => {
  vi.useFakeTimers({ toFake: ['Date'] });
  vi.setSystemTime(TODAY);
});
afterEach(() => vi.useRealTimers());

describe('dueDisplay', () => {
  it('a set date reads friendly, amber when due today', () => {
    expect(dueDisplay(item({ dueDate: '2026-11-13' }), TODAY)).toMatchObject({
      label: 'Tomorrow',
      title: 'Friday, November 13, 2026',
      urgent: false,
    });
    expect(dueDisplay(item({ dueDate: '2026-11-12' }), TODAY).urgent).toBe(true);
    expect(dueDisplay(item({ dueDate: '2026-11-12', status: 'done' as never }), TODAY).urgent).toBe(false);
  });

  it('a hint resolves against the day the item was created', () => {
    // Created Monday Nov 2: "next Tuesday" = Nov 10, two days ago.
    const r = dueDisplay(
      item({ dueHint: 'by next Tuesday', createdAt: new Date(2026, 10, 2, 15).toISOString() }),
      TODAY,
    );
    expect(r.label).toBe('Nov 10');
    expect(r.urgent).toBe(false);
    expect(r.title).toContain('Tuesday, November 10, 2026');
  });

  it('an unresolvable hint shows verbatim; nothing shows nothing', () => {
    expect(dueDisplay(item({ dueHint: 'before freeze' }), TODAY).label).toBe('before freeze');
    expect(dueDisplay(item(), TODAY).label).toBeNull();
  });
});

describe('DueDateControl picker', () => {
  function open(over: Partial<ActionItem> = {}, props: Record<string, unknown> = {}) {
    const onSet = vi.fn();
    render(<DueDateControl item={item(over)} onSetDueDate={onSet} {...props} />);
    fireEvent.click(screen.getByRole('button', { name: /due date|^Due /i }));
    return onSet;
  }

  it('empty labels: quiet "Set date", detail "Set due date"', () => {
    const { unmount } = render(<DueDateControl item={item()} onSetDueDate={vi.fn()} quietWhenEmpty />);
    expect(screen.getByText('Set date')).toBeTruthy();
    unmount();
    render(<DueDateControl item={item()} onSetDueDate={vi.fn()} size="body" />);
    expect(screen.getByText('Set due date')).toBeTruthy();
  });

  it('typing previews the date and Enter applies it', () => {
    const onSet = open();
    const input = screen.getByLabelText('Type a due date');
    fireEvent.change(input, { target: { value: 'by next wed' } });
    expect(screen.getByText('Wednesday, November 18, 2026', { exact: false })).toBeTruthy();
    fireEvent.submit(input.closest('form')!);
    expect(onSet).toHaveBeenCalledWith(expect.objectContaining({ id: 'a' }), '2026-11-18');
  });

  it('an unknown phrase says so and Enter does nothing', () => {
    const onSet = open();
    const input = screen.getByLabelText('Type a due date');
    fireEvent.change(input, { target: { value: 'whenever' } });
    expect(screen.getByText('Not a date I understand')).toBeTruthy();
    fireEvent.submit(input.closest('form')!);
    expect(onSet).not.toHaveBeenCalled();
  });

  it('quick picks: Tomorrow and Next week (the Monday after today)', () => {
    let onSet = open();
    fireEvent.click(screen.getByRole('button', { name: 'Tomorrow' }));
    expect(onSet.mock.calls[0]![1]).toBe('2026-11-13');
    cleanup();
    onSet = open({ dueHint: null });
    fireEvent.click(screen.getByRole('button', { name: 'Next week' }));
    expect(onSet.mock.calls[0]![1]).toBe('2026-11-16');
  });

  it('clicking a day applies it; Clear date clears', () => {
    let onSet = open();
    fireEvent.click(screen.getByRole('button', { name: 'Friday, November 20, 2026' }));
    expect(onSet.mock.calls[0]![1]).toBe('2026-11-20');
    expect(screen.queryByRole('button', { name: 'Clear date' })).toBeNull();

    cleanup();
    onSet = open({ dueDate: '2026-11-20' });
    expect(screen.getByRole('gridcell', { selected: true }).textContent).toBe('20');
    fireEvent.click(screen.getByRole('button', { name: 'Clear date' }));
    expect(onSet).toHaveBeenCalledWith(expect.anything(), null);
  });

  it('the month grid steps months and starts on Monday', () => {
    open();
    expect(screen.getByText('November 2026')).toBeTruthy();
    // Nov 1 2026 is a Sunday, so the grid opens on Monday Oct 26.
    const cells = screen.getAllByRole('gridcell');
    expect(cells[0]!.textContent).toBe('26');
    fireEvent.click(screen.getByRole('button', { name: 'Next month' }));
    expect(screen.getByText('December 2026')).toBeTruthy();
  });
});
