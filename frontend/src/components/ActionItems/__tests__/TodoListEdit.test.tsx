import React from 'react';
import { describe, it, expect, vi } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(async () => []) }));

import { TodoList } from '@/components/ActionItems/TodoList';
import type { TodoRow } from '@/types';

const row = {
  id: 'a1',
  description: 'Send the deck to Maya',
  status: 'open',
  source: 'manual',
  triage: 'listed',
  sourceCount: 0,
} as unknown as TodoRow;

describe('TodoList inline edit', () => {
  it('a click inside the edit field (to move the cursor) keeps editing', () => {
    const onCommitEdit = vi.fn();
    render(
      <TodoList
        rows={[row]}
        ariaLabel="List"
        nameById={new Map()}
        showTopic={false}
        onSetStatus={vi.fn()}
        onCommitEdit={onCommitEdit}
        onSetDueDate={vi.fn()}
        onDelete={vi.fn()}
        onOpen={vi.fn()}
        onToggleHighlight={vi.fn()}
      />,
    );
    const title = screen.getByRole('button', { name: 'Send the deck to Maya' });
    fireEvent.click(title); // select
    fireEvent.click(screen.getByRole('button', { name: 'Send the deck to Maya' })); // edit
    const field = screen.getByLabelText('Edit to-do');
    field.focus();
    fireEvent.mouseDown(field);
    fireEvent.click(field);
    expect(screen.getByLabelText('Edit to-do')).toBeInTheDocument();
    expect(document.activeElement).toBe(screen.getByLabelText('Edit to-do'));
    expect(onCommitEdit).not.toHaveBeenCalled();
  });
});
