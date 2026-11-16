import React from 'react';
import { afterEach, describe, it, expect, vi } from 'vitest';
import { render, screen, fireEvent, within } from '@testing-library/react';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(async () => []) }));

import { GroupedTodoList } from '@/components/ActionItems/GroupedTodoList';
import type { TodoRow, TopicSummary } from '@/types';

// specs/0090 — "Group by topic" on the To-Do list.

const row = (id: string, topicId: string | null) =>
  ({ id, description: `Item ${id}`, status: 'open', source: 'manual', triage: 'listed', sourceCount: 0, topicId }) as unknown as TodoRow;

const topic = (id: string, name: string, sortOrder: number, status: 'open' | 'closed' = 'open'): TopicSummary => ({
  id, name, description: '', status, lastActivity: null, pendingCount: 0, meetingCount: 0,
  sortOrder, starred: false, hideFromTodo: false,
});

function setup(excludeFromOrder?: Set<string>) {
  const reorderWithin = vi.fn();
  const onOpenTopic = vi.fn();
  const onMoveToTopic = vi.fn();
  const onReorderTopics = vi.fn();
  render(
    <GroupedTodoList
      rows={[row('1', 'atlas'), row('2', null), row('3', 'beta'), row('4', 'atlas'), row('5', 'atlas')]}
      allTopics={[topic('atlas', 'Atlas', 1), topic('beta', 'Beta', 0), topic('old', 'Old', 2, 'closed')]}
      reorderWithin={reorderWithin}
      onMoveToTopic={onMoveToTopic}
      onReorderTopics={onReorderTopics}
      excludeFromOrder={excludeFromOrder}
      onOpenTopic={onOpenTopic}
      nameById={new Map()}
      onSetStatus={vi.fn()}
      onCommitEdit={vi.fn()}
      onSetDueDate={vi.fn()}
      onDelete={vi.fn()}
      onOpen={vi.fn()}
      onToggleHighlight={vi.fn()}
    />,
  );
  return { reorderWithin, onOpenTopic, onMoveToTopic, onReorderTopics };
}

describe('GroupedTodoList', () => {
  it('one section per topic in topic order, then No Topic; no empty sections', () => {
    setup();
    const lists = screen.getAllByRole('listbox').map((l) => l.getAttribute('aria-label'));
    expect(lists).toEqual(['To-Do list: Beta', 'To-Do list: Atlas', 'To-Do list: No Topic']);
    const atlas = screen.getByRole('listbox', { name: 'To-Do list: Atlas' });
    expect(within(atlas).getAllByText(/^Item /).map((n) => n.textContent)).toEqual(['Item 1', 'Item 4', 'Item 5']);
  });

  it('a heading opens its Topic', () => {
    const { onOpenTopic } = setup();
    fireEvent.click(screen.getByRole('button', { name: /^Atlas/ }));
    expect(onOpenTopic).toHaveBeenCalledWith('atlas');
  });

  it("reorders within a section, sending only that section's ids", () => {
    const { reorderWithin } = setup();
    const atlas = screen.getByRole('listbox', { name: 'To-Do list: Atlas' });
    fireEvent.keyDown(within(atlas).getAllByRole('button', { name: 'Drag to reorder' })[2], { key: 'ArrowUp' });
    expect(reorderWithin).toHaveBeenCalledWith(['1', '5', '4']);
  });

  it('stops at the section edge and leaves ticked-off rows out of the saved order', () => {
    const { reorderWithin } = setup(new Set(['1']));
    const atlas = screen.getByRole('listbox', { name: 'To-Do list: Atlas' });
    const grips = within(atlas).getAllByRole('button', { name: 'Drag to reorder' });
    fireEvent.keyDown(grips[0], { key: 'ArrowUp' }); // already first: nothing
    // Moving 4 below ticked-off 1 changes nothing that is saved.
    fireEvent.keyDown(grips[1], { key: 'ArrowUp' });
    expect(reorderWithin).not.toHaveBeenCalled();
    fireEvent.keyDown(grips[2], { key: 'ArrowUp' });
    expect(reorderWithin).toHaveBeenCalledWith(['5', '4']);
  });
});

/** Drag `grip` and release it over `target` (jsdom has no layout: stub the hit test). */
function dragTo(grip: HTMLElement, target: Element) {
  document.elementFromPoint = vi.fn(() => target) as unknown as typeof document.elementFromPoint;
  fireEvent.pointerDown(grip, { button: 0, pointerId: 1 });
  fireEvent.pointerMove(grip, { pointerId: 1 });
  fireEvent.pointerUp(grip, { pointerId: 1 });
}

describe('GroupedTodoList drag across sections (specs/0090)', () => {
  afterEach(() => {
    // @ts-expect-error jsdom has no elementFromPoint; remove the stub
    delete document.elementFromPoint;
  });

  const rowEl = (text: string) => screen.getByText(text).closest('[data-reorder-id]')!;
  const gripOf = (listName: string, i: number) =>
    within(screen.getByRole('listbox', { name: listName })).getAllByRole('button', { name: 'Drag to reorder' })[i];

  it('a row dropped on another section\'s row re-tags it there, at that place', () => {
    const { onMoveToTopic, reorderWithin } = setup();
    dragTo(gripOf('To-Do list: No Topic', 0), rowEl('Item 4'));
    expect(onMoveToTopic).toHaveBeenCalledWith('2', 'atlas', ['1', '2', '4', '5']);
    expect(reorderWithin).not.toHaveBeenCalled();
  });

  it('a row dropped on a section heading goes to the end of that section', () => {
    const { onMoveToTopic } = setup();
    dragTo(gripOf('To-Do list: Atlas', 0), screen.getByRole('button', { name: /^Beta/ }));
    expect(onMoveToTopic).toHaveBeenCalledWith('1', 'beta', ['3', '1']);
  });

  it('a row dropped on No Topic clears its Topic (null)', () => {
    const { onMoveToTopic } = setup();
    dragTo(gripOf('To-Do list: Beta', 0), rowEl('Item 2'));
    expect(onMoveToTopic).toHaveBeenCalledWith('3', null, ['3', '2']);
  });

  it('a drop inside its own section still just reorders', () => {
    const { onMoveToTopic, reorderWithin } = setup();
    dragTo(gripOf('To-Do list: Atlas', 2), rowEl('Item 1'));
    expect(reorderWithin).toHaveBeenCalledWith(['5', '1', '4']);
    expect(onMoveToTopic).not.toHaveBeenCalled();
  });

  it('a topic heading drags to reorder the Topic sections; No Topic has no grip', () => {
    const { onReorderTopics, onMoveToTopic } = setup();
    const headingGrip = (name: string) =>
      within(screen.getByRole('heading', { name: new RegExp(`^${name}`) })).getByRole('button', { name: 'Drag to reorder' });
    expect(within(screen.getByRole('heading', { name: /^No Topic/ })).queryByRole('button', { name: 'Drag to reorder' })).toBeNull();
    dragTo(headingGrip('Atlas'), rowEl('Item 3'));
    expect(onReorderTopics).toHaveBeenCalledWith(['atlas', 'beta']);
    expect(onMoveToTopic).not.toHaveBeenCalled();
  });
});
