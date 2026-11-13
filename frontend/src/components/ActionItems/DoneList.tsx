'use client';

/**
 * DoneList (specs/0088) — completed to-dos grouped by Monday-start local week: "This
 * week", "Last week", then "Week of Nov 2". Rows are the shared `ActionItemRow`, so the
 * checkbox reopens an item. Input order (newest first from the backend) is preserved
 * within and across groups.
 */

import { useMemo } from 'react';
import { ActionItemRow } from './ActionItemRow';
import type { AssigneeCandidate } from './AssigneePicker';
import { assigneeLabel } from '@/lib/action-items';
import type { ActionItem, ActionItemStatus, TodoRow } from '@/types';
import type { AssigneeSelection } from '@/lib/action-items';

/** Local midnight of the Monday that starts `d`'s week. */
export function weekStart(d: Date): Date {
  const mondayIndex = (d.getDay() + 6) % 7;
  return new Date(d.getFullYear(), d.getMonth(), d.getDate() - mondayIndex);
}

export interface DoneGroup {
  key: string;
  title: string;
  items: TodoRow[];
}

/** Group completed rows by the week they were completed (falling back to `updatedAt`). */
export function groupDoneByWeek(rows: TodoRow[], now: Date): DoneGroup[] {
  const thisWeek = weekStart(now).getTime();
  const lastWeek = new Date(
    new Date(thisWeek).getFullYear(),
    new Date(thisWeek).getMonth(),
    new Date(thisWeek).getDate() - 7,
  ).getTime();
  const groups = new Map<number, DoneGroup>();
  for (const row of rows) {
    const when = new Date(row.completedAt ?? row.updatedAt);
    const start = Number.isNaN(when.getTime()) ? thisWeek : weekStart(when).getTime();
    let group = groups.get(start);
    if (!group) {
      const title =
        start === thisWeek
          ? 'This week'
          : start === lastWeek
            ? 'Last week'
            : `Week of ${new Date(start).toLocaleDateString([], { month: 'short', day: 'numeric' })}`;
      group = { key: String(start), title, items: [] };
      groups.set(start, group);
    }
    group.items.push(row);
  }
  return [...groups.entries()].sort((a, b) => b[0] - a[0]).map(([, g]) => g);
}

interface DoneListProps {
  rows: TodoRow[];
  nameById: Map<string, string>;
  candidates: AssigneeCandidate[];
  onSetStatus: (item: ActionItem, status: ActionItemStatus) => void | Promise<void>;
  onEditDescription: (item: ActionItem, description: string) => void | Promise<void>;
  onAssign: (item: ActionItem, selection: AssigneeSelection) => void | Promise<void>;
  onDelete: (item: ActionItem) => void | Promise<void>;
  onOpen: (item: ActionItem) => void;
}

export function DoneList({
  rows,
  nameById,
  candidates,
  onSetStatus,
  onEditDescription,
  onAssign,
  onDelete,
  onOpen,
}: DoneListProps) {
  const groups = useMemo(() => groupDoneByWeek(rows, new Date()), [rows]);
  return (
    <div className="flex flex-col gap-5 px-1">
      {groups.map((group) => (
        <section key={group.key} aria-label={group.title}>
          <h3 className="u-section-label px-2 py-1.5">{group.title}</h3>
          <div className="mt-0.5 flex flex-col gap-0.5">
            {group.items.map((item) => (
              <ActionItemRow
                key={item.id}
                item={item}
                assigneeName={assigneeLabel(item, nameById)}
                candidates={candidates}
                nameById={nameById}
                onSetStatus={onSetStatus}
                onEditDescription={onEditDescription}
                onAssign={onAssign}
                onDelete={onDelete}
                onOpen={onOpen}
              />
            ))}
          </div>
        </section>
      ))}
    </div>
  );
}
