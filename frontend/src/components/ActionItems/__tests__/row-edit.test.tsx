import { describe, it, expect, vi } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';
import { ActionItemRow } from '../ActionItemRow';
import type { ActionItem } from '@/types';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(async () => []) }));

const item = { id: 'a', description: 'Send deck', status: 'open', source: 'manual', triage: 'listed' } as unknown as ActionItem;

describe('ActionItemRow edit', () => {
  it('Esc then the blur on unmount never saves the cancelled edit', async () => {
    const onEdit = vi.fn();
    render(
      <ActionItemRow item={item} assigneeName={null} candidates={[]} onSetStatus={vi.fn()}
        onEditDescription={onEdit} onAssign={vi.fn()} onDelete={vi.fn()} />,
    );
    fireEvent.click(screen.getByTitle('Edit description'));
    const input = screen.getByLabelText('Edit action item description');
    fireEvent.change(input, { target: { value: 'Different' } });
    fireEvent.keyDown(input, { key: 'Escape' });
    fireEvent.blur(input);
    expect(onEdit).not.toHaveBeenCalled();
  });

  it('Enter then blur saves once', () => {
    const onEdit = vi.fn();
    render(
      <ActionItemRow item={item} assigneeName={null} candidates={[]} onSetStatus={vi.fn()}
        onEditDescription={onEdit} onAssign={vi.fn()} onDelete={vi.fn()} />,
    );
    fireEvent.click(screen.getByTitle('Edit description'));
    const input = screen.getByLabelText('Edit action item description');
    fireEvent.change(input, { target: { value: 'Different' } });
    fireEvent.keyDown(input, { key: 'Enter' });
    fireEvent.blur(input);
    expect(onEdit).toHaveBeenCalledTimes(1);
  });

  it('a trailing (date) on an edited title sets the due date and leaves the title', () => {
    const onEdit = vi.fn();
    const onSetDueDate = vi.fn();
    render(
      <ActionItemRow item={item} assigneeName={null} candidates={[]} onSetStatus={vi.fn()}
        onEditDescription={onEdit} onAssign={vi.fn()} onDelete={vi.fn()} onSetDueDate={onSetDueDate} />,
    );
    fireEvent.click(screen.getByTitle('Edit description'));
    const input = screen.getByLabelText('Edit action item description');
    fireEvent.change(input, { target: { value: 'Send the deck (Jan 5)' } });
    fireEvent.keyDown(input, { key: 'Enter' });
    expect(onEdit).toHaveBeenCalledWith(item, 'Send the deck');
    expect(onSetDueDate).toHaveBeenCalledTimes(1);
    expect(onSetDueDate.mock.calls[0]![1]).toMatch(/^\d{4}-01-05$/);
  });
});
