import { describe, it, expect } from 'vitest';
import {
  affectedByBulk,
  assigneeLabel,
  reorderIds,
  updateAssigneeArgs,
} from '@/lib/action-items';
import type { ActionItem, ActionItemWithMeeting } from '@/types';

// specs/0034 + 0088 — the action-item helpers, pure and IPC-free. Locks: (a) assignee
// picker selections map to the exact `api_update_action_item` args, (b) drag reordering,
// (c) bulk-dismiss selectors, (d) assignee display resolution: Me → directory name → raw.

function item(overrides: Partial<ActionItemWithMeeting>): ActionItemWithMeeting {
  return {
    id: 'ai-1',
    meetingId: 'm1',
    description: 'Send the deck',
    assigneePersonId: null,
    assigneeIsSelf: false,
    assigneeRaw: null,
    dueHint: null,
    dueDate: null,
    status: 'open',
    source: 'extracted',
    userEdited: false,
    contentKey: 'ck',
    createdAt: '2026-07-01T10:00:00Z',
    updatedAt: '2026-07-01T10:00:00Z',
    completedAt: null,
    sortOrder: null,
    triage: 'suggested',
    adoptedAt: null,
    notesMarkdown: null,
    meetingTitle: 'Weekly sync',
    meetingCreatedAt: '2026-07-01T09:00:00Z',
    ...overrides,
  };
}

describe('updateAssigneeArgs', () => {
  it('maps each selection kind to the api_update_action_item args', () => {
    expect(updateAssigneeArgs('ai-1', { kind: 'me' })).toEqual({
      id: 'ai-1',
      assigneeIsSelf: true,
    });
    expect(updateAssigneeArgs('ai-1', { kind: 'person', personId: 'p2' })).toEqual({
      id: 'ai-1',
      assigneePersonId: 'p2',
    });
    expect(updateAssigneeArgs('ai-1', { kind: 'raw', name: 'Bob' })).toEqual({
      id: 'ai-1',
      assigneeRaw: 'Bob',
    });
    expect(updateAssigneeArgs('ai-1', { kind: 'clear' })).toEqual({
      id: 'ai-1',
      assigneeClear: true,
    });
  });
});

describe('reorderIds (specs/0038 WS1.b drag)', () => {
  it('moves the dragged id into the target slot', () => {
    expect(reorderIds(['a', 'b', 'c'], 'c', 'a')).toEqual(['c', 'a', 'b']);
    expect(reorderIds(['a', 'b', 'c'], 'a', 'c')).toEqual(['b', 'c', 'a']);
  });

  it('is a no-op for the same id or a missing id', () => {
    expect(reorderIds(['a', 'b'], 'a', 'a')).toEqual(['a', 'b']);
    expect(reorderIds(['a', 'b'], 'z', 'a')).toEqual(['a', 'b']);
  });
});

describe('affectedByBulk (specs/0038 WS1.e)', () => {
  const items = [
    item({ id: 'mine', assigneeIsSelf: true }),
    item({ id: 'alice', assigneePersonId: 'p1' }),
    item({ id: 'unassigned' }),
    item({ id: 'alice-done', assigneePersonId: 'p1', status: 'completed' }),
  ];

  it('notSelf: every OPEN item not assigned to me (covers unassigned/others)', () => {
    expect(affectedByBulk(items, { mode: 'notSelf' })).toEqual(['alice', 'unassigned']);
  });

  it('person: OPEN items assigned to that person only', () => {
    expect(affectedByBulk(items, { mode: 'person', personId: 'p1' })).toEqual(['alice']);
  });

  it('ids: passes the explicit set through', () => {
    expect(affectedByBulk(items, { mode: 'ids', ids: ['x', 'y'] })).toEqual(['x', 'y']);
  });
});

describe('assigneeLabel', () => {
  const names = new Map([['p1', 'Alice Chen']]);

  it('prefers the self flag', () => {
    const it_ = item({ assigneeIsSelf: true, assigneePersonId: 'p1' }) as ActionItem;
    expect(assigneeLabel(it_, names)).toBe('Me');
  });

  it('resolves a person id through the directory', () => {
    expect(assigneeLabel(item({ assigneePersonId: 'p1' }), names)).toBe('Alice Chen');
  });

  it('falls back to the raw name for a dangling person id (person deleted)', () => {
    expect(
      assigneeLabel(item({ assigneePersonId: 'gone', assigneeRaw: 'Alice' }), names),
    ).toBe('Alice');
  });

  it('shows the raw extracted name when unresolved, null when unassigned', () => {
    expect(assigneeLabel(item({ assigneeRaw: 'Bob' }), names)).toBe('Bob');
    expect(assigneeLabel(item({}), names)).toBeNull();
  });
});
