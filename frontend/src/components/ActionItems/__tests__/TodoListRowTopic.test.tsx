import { fireEvent, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import type { ActionItem } from '@/types';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(async () => []) }));

import { TodoListRow } from '../TodoListRow';

const item = {
  id: 'a1',
  description: 'Send the deck',
  status: 'open',
  source: 'manual',
  triage: 'listed',
  dueDate: '2026-11-20',
  topicId: 'topic-1',
  topicName: 'Atlas CRM rebuild',
} as unknown as ActionItem;

const noop = vi.fn();
const props = {
  nameById: new Map<string, string>(),
  onSelect: noop,
  onStartEdit: noop,
  onCommitEdit: noop,
  onCancelEdit: noop,
  onOpen: noop,
  onSetStatus: noop,
  onSetDueDate: noop,
  onDelete: noop,
  onToggleHighlight: noop,
  onMoveToTop: noop,
};

describe('TodoListRow topic chip', () => {
  it('renders the chip before the Due cell and routes to the Topic on click', () => {
    const onOpenTopic = vi.fn();
    render(<TodoListRow {...props} item={item} onOpenTopic={onOpenTopic} />);
    const chip = screen.getByRole('button', { name: 'Open Topic Atlas CRM rebuild' });
    const row = chip.closest('[role="option"]')!;
    const cells = Array.from(row.children);
    const chipCell = cells.findIndex((c) => c.contains(chip));
    const dueCell = cells.findIndex((c) => c.textContent?.includes('Nov') || c.querySelector('[aria-label*="ue"]'));
    expect(chipCell).toBeGreaterThan(0);
    expect(chipCell).toBeLessThan(dueCell);
    fireEvent.click(chip);
    expect(onOpenTopic).toHaveBeenCalledWith('topic-1');
  });

  it('renders no chip for an untagged item', () => {
    render(<TodoListRow {...props} item={{ ...item, topicId: null, topicName: null }} />);
    expect(screen.queryByRole('button', { name: /Open Topic/ })).toBeNull();
  });

  it('without the Topic column the row has no topic cell and the narrower grid', () => {
    render(<TodoListRow {...props} item={item} showTopic={false} />);
    expect(screen.queryByRole('button', { name: /Open Topic/ })).toBeNull();
    const row = screen.getByRole('option');
    expect(row.className).not.toMatch(/8rem/);
    expect(row.children).toHaveLength(4); // checkbox, title, due, actions: four columns
  });
});

describe('TodoListRow compact', () => {
  it('is checkbox · title · actions in a row, with the due date under the title', () => {
    render(<TodoListRow {...props} item={item} showTopic={false} compact />);
    const row = screen.getByRole('option');
    expect(row.className).toMatch(/\bflex\b/);
    expect(row.className).not.toMatch(/grid/);
    const [check, body, actions] = Array.from(row.children);
    expect(check.querySelector('[role="checkbox"]')).not.toBeNull();
    expect(body).toContainElement(screen.getByText('Send the deck'));
    expect(body.querySelector('[aria-label*="ue"]')).not.toBeNull(); // the due control
    expect(actions).toContainElement(screen.getByRole('button', { name: 'Options for "Send the deck"' }));
  });

  it('an undated row has no second line', () => {
    render(<TodoListRow {...props} item={{ ...item, dueDate: null }} showTopic={false} compact />);
    const body = screen.getByRole('option').children[1];
    expect(body.querySelector('[aria-label*="ue"]')).toBeNull();
  });
});

describe('TodoListRow "Set due date…"', () => {
  it('opens the date picker from the row menu, even on an undated compact row', async () => {
    const onSetDueDate = vi.fn();
    render(
      <TodoListRow
        {...props}
        onSetDueDate={onSetDueDate}
        item={{ ...item, dueDate: null }}
        showTopic={false}
        compact
      />,
    );
    expect(screen.queryByPlaceholderText(/Type a date/)).toBeNull();
    const trigger = screen.getByRole('button', { name: 'Options for "Send the deck"' });
    await userEvent.click(trigger);
    await userEvent.click(await screen.findByRole('menuitem', { name: /Set due date/ }));
    expect(await screen.findByPlaceholderText(/Type a date/)).toBeInTheDocument();
  });

  it('reads "Change due date…" when the item has one', async () => {
    render(<TodoListRow {...props} item={item} showTopic={false} compact />);
    const trigger = screen.getByRole('button', { name: 'Options for "Send the deck"' });
    await userEvent.click(trigger);
    expect(await screen.findByRole('menuitem', { name: /Change due date/ })).toBeInTheDocument();
  });
});
