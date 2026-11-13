'use client';

/**
 * The open topics under the Topics row (specs/0089): up to six, most recent activity first,
 * each with its pending-suggestion count, then "+N more" when there are others. Expanded
 * sidebar only; the collapsed rail has no room for names.
 */

import React, { useMemo } from 'react';
import { useRouter } from 'next/navigation';
import { Badge } from '@/components/ui/badge';
import { useTopics } from '@/hooks/useTopics';
import { pendingLabel } from '@/lib/topics';
import { SIDEBAR_ICON_SLOT, SIDEBAR_ROW } from './row';
import { cn } from '@/lib/utils';

const MAX_LISTED = 6;

const ROW_CLASS = cn(
  SIDEBAR_ROW,
  'h-8 transition-colors hover:bg-key focus:outline-none focus-visible:ring-2 focus-visible:ring-ring',
);

export function TopicsNavList({ collapsed }: { collapsed: boolean }) {
  const router = useRouter();
  const { data } = useTopics('open');
  const sorted = useMemo(
    () =>
      [...(data ?? [])].sort((a, b) =>
        (b.lastActivity ?? '').localeCompare(a.lastActivity ?? ''),
      ),
    [data],
  );
  if (collapsed || sorted.length === 0) return null;
  const shown = sorted.slice(0, MAX_LISTED);
  const more = sorted.length - shown.length;
  return (
    <div data-topics-list>
      {shown.map((t) => (
        <button // ui-gate: allow sidebar sub-row: shares the rail's icon-column geometry (row.ts); ListRow's padding would shift the label
          key={t.id}
          type="button"
          aria-label={t.pendingCount > 0 ? `${t.name}, ${pendingLabel(t.pendingCount)}` : t.name}
          onClick={() => router.push(`/topics/topic?id=${encodeURIComponent(t.id)}`)}
          className={cn(ROW_CLASS, 'text-engrave hover:text-foreground')}
        >
          <span aria-hidden className={SIDEBAR_ICON_SLOT} />
          <span className="min-w-0 flex-1 truncate pr-2 text-left text-sm">{t.name}</span>
          {t.pendingCount > 0 ? (
            <Badge
              tone="brand"
              title={pendingLabel(t.pendingCount)}
              aria-label={pendingLabel(t.pendingCount)}
              className="mr-3.5"
            >
              {t.pendingCount}
            </Badge>
          ) : null}
        </button>
      ))}
      {more > 0 ? (
        <button // ui-gate: allow sidebar sub-row: same geometry as the topic rows above
          type="button"
          aria-label={`+${more} more`}
          onClick={() => router.push('/topics')}
          className={cn(ROW_CLASS, 'text-meta text-muted-foreground hover:text-foreground')}
        >
          <span aria-hidden className={SIDEBAR_ICON_SLOT} />
          <span className="min-w-0 flex-1 truncate pr-3.5 text-left">{`+${more} more`}</span>
        </button>
      ) : null}
    </div>
  );
}
