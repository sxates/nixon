'use client';

/**
 * One section of the Topics page (open, or archived): a row per topic with a drag grip in
 * the gutter and a star toggle (specs/0090). Dragging reorders within the section only; a
 * drop on the other section's rows changes nothing. The caller owns the order and stars.
 */

import { Badge } from '@/components/ui/badge';
import { DeckIcon } from '@/components/ui/deck-icon';
import { IconButton } from '@/components/ui/icon-button';
import { Star } from '@/components/ui/icons';
import { ListRow } from '@/components/ui/list-row';
import { ReorderGrip } from '@/components/ActionItems/ReorderGrip';
import { REORDER_ATTR, useDragReorder } from '@/hooks/useDragReorder';
import { formatDecisionDate, pendingLabel } from '@/lib/topics';
import { cn } from '@/lib/utils';
import type { TopicSummary } from '@/types';

export interface TopicIndexRowsProps {
  topics: TopicSummary[];
  onOpen: (id: string) => void;
  /** The section's ids in their new order. */
  onReorder: (ids: string[]) => void;
  onToggleStar: (topic: TopicSummary) => void;
}

export function TopicIndexRows({ topics, onOpen, onReorder, onToggleStar }: TopicIndexRowsProps) {
  const ids = topics.map((t) => t.id);
  const { draggingId, overId, gripProps } = useDragReorder(ids, onReorder);
  return (
    <div className="flex flex-col">
      {topics.map((t) => {
        const when = t.lastActivity ? formatDecisionDate(t.lastActivity) : '';
        return (
          <div
            key={t.id}
            {...{ [REORDER_ATTR]: t.id }}
            className={cn(
              'group relative rounded-lg transition-shadow',
              draggingId === t.id && 'opacity-50',
              overId === t.id && draggingId && draggingId !== t.id && 'ring-2 ring-brand-edge',
            )}
          >
            {/* In the gutter, as on the To-do list, so the rows keep the header's width. */}
            <span className="absolute -left-4 top-0 flex h-full items-center">
              <ReorderGrip {...gripProps(t.id)} dragging={draggingId === t.id} />
            </span>
            <ListRow
              ruled
              title={t.name}
              meta={[
                t.description.split('\n')[0],
                when && `Last activity ${when}`,
                t.pendingCount > 0 && pendingLabel(t.pendingCount),
              ]
                .filter(Boolean)
                .join(' · ')}
              trailing={
                <>
                  {t.pendingCount > 0 ? (
                    <Badge
                      tone="brand"
                      title={pendingLabel(t.pendingCount)}
                      aria-label={pendingLabel(t.pendingCount)}
                    >
                      {t.pendingCount}
                    </Badge>
                  ) : null}
                  {/* Starred rows keep the filled star; unstarred reveal it on hover. */}
                  <IconButton
                    variant="ghost"
                    label={t.starred ? `Unstar ${t.name}` : `Star ${t.name}`}
                    aria-pressed={t.starred}
                    onClick={() => onToggleStar(t)}
                    icon={
                      <DeckIcon icon={Star} size={14} className={t.starred ? 'fill-current' : undefined} />
                    }
                    className={
                      t.starred
                        ? 'text-star hover:text-star'
                        : 'opacity-0 focus-visible:opacity-100 group-hover/row:opacity-100'
                    }
                  />
                </>
              }
              onClick={() => onOpen(t.id)}
            />
          </div>
        );
      })}
    </div>
  );
}
