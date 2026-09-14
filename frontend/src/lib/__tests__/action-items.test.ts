import { describe, it, expect } from 'vitest';
import {
  affectedByBulk,
  assigneeLabel,
  buildListArgs,
  groupActionItems,
  reorderIds,
  sortActionItems,
  updateAssigneeArgs,
  UNATTACHED_GROUP_TITLE,
} from '@/lib/action-items';
import type { ActionItem, ActionItemWithMeeting } from '@/types';

// specs/0034 — the task hub's core logic, pure and IPC-free. Locks: (a) the filter
// state maps to the exact `api_list_action_items` args, (b) grouping is by source
// meeting newest-first with standalone items in a trailing "Unattached" group,
// (c) assignee display resolution: Me → directory name → raw fallback.

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
    meetingTitle: 'Weekly sync',
    meetingCreatedAt: '2026-07-01T09:00:00Z',
    ...overrides,
  };
}

describe('buildListArgs (specs/0034 hub filters)', () => {
  it('maps Anyone to no person constraint', () => {
    expect(buildListArgs('open', { kind: 'anyone' })).toEqual({
      status: 'open',
      personId: null,
      mineOnly: false,
    });
  });

  it('maps Me to mineOnly', () => {
    expect(buildListArgs('completed', { kind: 'me' })).toEqual({
      status: 'completed',
      personId: null,
      mineOnly: true,
    });
  });

  it('maps a person to their id', () => {
    expect(buildListArgs('dismissed', { kind: 'person', personId: 'p7' })).toEqual({
      status: 'dismissed',
      personId: 'p7',
      mineOnly: false,
    });
  });
});

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

describe('groupActionItems (specs/0034 hub grouping)', () => {
  it('groups by meeting, newest meeting first, unattached last', () => {
    const items = [
      item({ id: 'a', meetingId: 'm-old', meetingTitle: 'Old planning', meetingCreatedAt: '2026-06-01T09:00:00Z' }),
      item({ id: 'b', meetingId: null, meetingTitle: null, meetingCreatedAt: null, source: 'manual' }),
      item({ id: 'c', meetingId: 'm-new', meetingTitle: 'New sync', meetingCreatedAt: '2026-07-01T09:00:00Z' }),
      item({ id: 'd', meetingId: 'm-old', meetingTitle: 'Old planning', meetingCreatedAt: '2026-06-01T09:00:00Z' }),
    ];

    const groups = groupActionItems(items);
    expect(groups.map((g) => g.title)).toEqual([
      'New sync',
      'Old planning',
      UNATTACHED_GROUP_TITLE,
    ]);
    expect(groups[0]!.meetingId).toBe('m-new');
    expect(groups[1]!.items.map((i) => i.id)).toEqual(['a', 'd']);
    expect(groups[2]!.meetingId).toBeNull();
    expect(groups[2]!.items.map((i) => i.id)).toEqual(['b']);
  });

  it('orders items within a group oldest-first by createdAt', () => {
    const items = [
      item({ id: 'later', createdAt: '2026-07-01T12:00:00Z' }),
      item({ id: 'earlier', createdAt: '2026-07-01T08:00:00Z' }),
    ];
    const [group] = groupActionItems(items);
    expect(group!.items.map((i) => i.id)).toEqual(['earlier', 'later']);
  });

  it('falls back to "Untitled meeting" for a blank title and returns [] for no items', () => {
    const [group] = groupActionItems([item({ meetingTitle: '  ' })]);
    expect(group!.title).toBe('Untitled meeting');
    expect(groupActionItems([])).toEqual([]);
  });
});

describe('sortActionItems (specs/0038 WS1.b)', () => {
  it('Manual: by sortOrder ascending, NULLs last then createdAt', () => {
    const items = [
      item({ id: 'null-late', sortOrder: null, createdAt: '2026-07-02T00:00:00Z' }),
      item({ id: 'ord-1', sortOrder: 1 }),
      item({ id: 'null-early', sortOrder: null, createdAt: '2026-07-01T00:00:00Z' }),
      item({ id: 'ord-0', sortOrder: 0 }),
    ];
    expect(sortActionItems(items, 'manual').map((i) => i.id)).toEqual([
      'ord-0',
      'ord-1',
      'null-early',
      'null-late',
    ]);
  });

  it('Due: soonest dueDate first, NULLs last', () => {
    const items = [
      item({ id: 'none', dueDate: null }),
      item({ id: 'late', dueDate: '2026-08-01' }),
      item({ id: 'soon', dueDate: '2026-07-10' }),
    ];
    expect(sortActionItems(items, 'due').map((i) => i.id)).toEqual([
      'soon',
      'late',
      'none',
    ]);
  });

  it('does not mutate the input array', () => {
    const items = [item({ id: 'b', sortOrder: 1 }), item({ id: 'a', sortOrder: 0 })];
    const before = items.map((i) => i.id);
    sortActionItems(items, 'manual');
    expect(items.map((i) => i.id)).toEqual(before);
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
