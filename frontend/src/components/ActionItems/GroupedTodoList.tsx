'use client';

/**
 * The To-do list grouped by topic (specs/0090): one section per topic in the Topics page's
 * order (open topics, then archived), then "No Topic"; empty sections are left out.
 *
 * - **Rows within a section**: each section is its own `TodoList`, so drag, ⌘↑↓ and ⌘1–3
 *   move an item within its section through the slot rule (`reorderWithin`): the section's
 *   items swap the places they hold on the flat list, and nothing else moves.
 * - **Rows across sections**: dropping a row on another section (one of its rows, its
 *   heading or its body) re-tags it to that section's topic — "No Topic" clears the topic —
 *   and places it at the drop point (`onMoveToTopic` gets the target section's ids in their
 *   new order, the moved item included).
 * - **Sections**: a topic heading has its own grip; dropping it above or below another
 *   section reorders the topics themselves (`onReorderTopics`, the section topic ids in
 *   their new order). "No Topic" stays last and has no grip.
 *
 * Keyboard selection (↑↓) stays inside a section; Tab moves to the next section's list.
 * Sections move by pointer only.
 */

import { useMemo } from 'react';
import { Button } from '@/components/ui/button';
import { DeckIcon } from '@/components/ui/deck-icon';
import { ChevronRight } from '@/components/ui/icons';
import { REORDER_ATTR, useDragReorder } from '@/hooks/useDragReorder';
import { groupByTopic } from '@/lib/todo-groups';
import { sameOrder } from '@/lib/topic-order';
import { cn } from '@/lib/utils';
import type { TodoRow, TopicSummary } from '@/types';
import { ReorderGrip } from './ReorderGrip';
import { TodoList, type TodoListProps } from './TodoList';

/** Marks a section; the value is its topic id, or "" for No Topic. */
export const GROUP_ATTR = 'data-todo-group';
/** Marks a draggable (topic) section for the heading grips. */
const SECTION_ATTR = 'data-reorder-group';

export interface GroupedTodoListProps
  extends Omit<TodoListProps, 'rows' | 'ariaLabel' | 'reorder' | 'showTopic' | 'className'> {
  rows: TodoRow[];
  /** Every topic (open and archived), for the section order and headings. */
  allTopics: TopicSummary[];
  /** Persist a section's new order (its ids only). */
  reorderWithin: (ids: string[]) => void;
  /** Re-tag a row dropped on another section; `targetIds` = that section in its new order. */
  onMoveToTopic: (id: string, topicId: string | null, targetIds: string[]) => void;
  /** The topic sections' ids in their new order. */
  onReorderTopics: (topicIds: string[]) => void;
  /** Ids whose rows show but sit outside the saved order (ticked off on this visit). */
  excludeFromOrder?: ReadonlySet<string>;
  onOpenTopic: (topicId: string) => void;
  className?: string;
}

export function GroupedTodoList({
  rows,
  allTopics,
  reorderWithin,
  onMoveToTopic,
  onReorderTopics,
  excludeFromOrder,
  onOpenTopic,
  className,
  ...listProps
}: GroupedTodoListProps) {
  const groups = useMemo(() => groupByTopic(rows, allTopics), [rows, allTopics]);
  const kept = (list: string[]) => list.filter((id) => !excludeFromOrder?.has(id));

  const sectionIds = groups.flatMap((g) => (g.topicId ? [g.topicId] : []));
  const sections = useDragReorder(
    sectionIds,
    (next) => {
      if (!sameOrder(next, sectionIds)) onReorderTopics(next);
    },
    { attr: SECTION_ATTR },
  );

  /** A row from `fromTopicId` released on `target`: move it if that's another section. */
  const dropOutside = (fromTopicId: string | null) => (draggedId: string, target: HTMLElement | null) => {
    const section = target?.closest<HTMLElement>(`[${GROUP_ATTR}]`);
    if (!section) return false;
    const toTopicId = section.getAttribute(GROUP_ATTR) || null;
    if (toTopicId === fromTopicId) return false;
    const group = groups.find((g) => g.topicId === toTopicId);
    if (!group) return false;
    const ids = group.rows.map((r) => r.id);
    const overRow = target?.closest<HTMLElement>(`[${REORDER_ATTR}]`)?.getAttribute(REORDER_ATTR);
    const at = overRow && ids.includes(overRow) ? ids.indexOf(overRow) : ids.length;
    const next = [...ids.slice(0, at), draggedId, ...ids.slice(at)];
    onMoveToTopic(draggedId, toTopicId, kept(next));
    return true;
  };

  return (
    <div className={className}>
      {groups.map((group, index) => {
        const ids = group.rows.map((r) => r.id);
        const reorder = (next: string[]) => {
          if (!sameOrder(kept(next), kept(ids))) reorderWithin(kept(next));
        };
        const headingId = `todo-group-${group.topicId ?? 'none'}`;
        const topicId = group.topicId;
        const draggingSection = topicId !== null && sections.draggingId === topicId;
        return (
          <section
            key={topicId ?? '∅'}
            aria-labelledby={headingId}
            {...{ [GROUP_ATTR]: topicId ?? '' }}
            {...(topicId ? { [SECTION_ATTR]: topicId } : {})}
            className={cn(
              'relative mt-4 rounded-lg transition-shadow first:mt-0',
              draggingSection && 'opacity-50',
              topicId &&
                sections.overId === topicId &&
                sections.draggingId &&
                sections.draggingId !== topicId &&
                'ring-2 ring-brand-edge',
            )}
          >
            <h2 id={headingId} className="group relative flex items-center gap-2 px-1 pb-1">
              {topicId ? (
                <>
                  <span className="absolute -left-4 top-0 flex h-full items-center">
                    <ReorderGrip {...sections.gripProps(topicId)} dragging={draggingSection} />
                  </span>
                  <Button variant="ghost" size="xs" className="px-2" onClick={() => onOpenTopic(topicId)}>
                    <span className="u-row-title">{group.name}</span>
                    <DeckIcon icon={ChevronRight} size={14} className="text-muted-foreground" />
                  </Button>
                </>
              ) : (
                <span className="u-row-title px-2">{group.name}</span>
              )}
              <span className="u-row-meta">{group.rows.length}</span>
            </h2>
            <TodoList
              {...listProps}
              rows={group.rows}
              ariaLabel={`To-Do list: ${group.name}`}
              showTopic={false}
              columnHeader={index === 0}
              onOpenTopic={onOpenTopic}
              reorder={reorder}
              onDropOutside={dropOutside(topicId)}
            />
          </section>
        );
      })}
    </div>
  );
}
