import { fireEvent, render, screen } from '@testing-library/react';
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
  it('renders the chip before the Due cell and routes to the topic on click', () => {
    const onOpenTopic = vi.fn();
    render(<TodoListRow {...props} item={item} onOpenTopic={onOpenTopic} />);
    const chip = screen.getByRole('button', { name: 'Open topic Atlas CRM rebuild' });
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
    expect(screen.queryByRole('button', { name: /Open topic/ })).toBeNull();
  });
});
