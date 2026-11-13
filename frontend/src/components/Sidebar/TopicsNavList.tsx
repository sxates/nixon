'use client';

/**
 * The Topics nav entry (specs/0089): the Topics row, then the open topics under it — up to
 * six, most recent activity first, each with its pending-suggestion count, then "+N more"
 * when there are others. The list shows only in the expanded sidebar; the collapsed rail
 * has no room for names.
 *
 * On a topic's page, that topic's row is the lit one (index bar, full-strength text) and the
 * Topics row stays dark, so one row is lit at a time. When the open topic isn't listed
 * (closed, past the first six, or the rail is collapsed) the Topics row lights instead.
 */

import React, { Suspense, useMemo } from 'react';
import { usePathname, useRouter, useSearchParams } from 'next/navigation';
import { Badge } from '@/components/ui/badge';
import { FolderKanban } from '@/components/ui/icons';
import { useTopics } from '@/hooks/useTopics';
import { pendingLabel } from '@/lib/topics';
import type { TopicSummary } from '@/types';
import { IndexBar, NavRow } from './SidebarRow';
import { SIDEBAR_ICON_SLOT, SIDEBAR_LABEL, SIDEBAR_ROW } from './row';
import { cn } from '@/lib/utils';

const MAX_LISTED = 6;

const ROW_CLASS = cn(
  SIDEBAR_ROW,
  'relative h-8 transition-colors hover:bg-key focus:outline-none focus-visible:ring-2 focus-visible:ring-ring',
);

const isTopicsPath = (p: string | null) => p?.startsWith('/topics') ?? false;

/** The open topics the sidebar lists, in order, plus how many more there are. */
function listed(data: TopicSummary[] | null): { shown: TopicSummary[]; more: number } {
  const sorted = [...(data ?? [])].sort((a, b) =>
    (b.lastActivity ?? '').localeCompare(a.lastActivity ?? ''),
  );
  return { shown: sorted.slice(0, MAX_LISTED), more: Math.max(0, sorted.length - MAX_LISTED) };
}

export function TopicsNavList({
  collapsed,
  topics,
  activeId = null,
}: {
  collapsed: boolean;
  topics: TopicSummary[] | null;
  /** The topic whose page is open, if any: its row is lit. */
  activeId?: string | null;
}) {
  const router = useRouter();
  const { shown, more } = useMemo(() => listed(topics), [topics]);
  if (collapsed || shown.length === 0) return null;
  return (
    <div data-topics-list>
      {shown.map((t) => {
        const active = t.id === activeId;
        return (
          <button // ui-gate: allow sidebar sub-row: shares the rail's icon-column geometry (row.ts); ListRow's padding would shift the label
            key={t.id}
            type="button"
            aria-label={t.pendingCount > 0 ? `${t.name}, ${pendingLabel(t.pendingCount)}` : t.name}
            aria-current={active ? 'page' : undefined}
            onClick={() => router.push(`/topics/topic?id=${encodeURIComponent(t.id)}`)}
            className={cn(ROW_CLASS, active ? 'text-foreground' : 'text-engrave hover:text-foreground')}
          >
            <IndexBar active={active} />
            <span aria-hidden className={SIDEBAR_ICON_SLOT} />
            <span className={cn(SIDEBAR_LABEL, 'pr-2')}>{t.name}</span>
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
        );
      })}
      {more > 0 ? (
        <button // ui-gate: allow sidebar sub-row: same geometry as the topic rows above
          type="button"
          aria-label={`+${more} more`}
          onClick={() => router.push('/topics')}
          className={cn(ROW_CLASS, 'text-engrave hover:text-foreground')}
        >
          <span aria-hidden className={SIDEBAR_ICON_SLOT} />
          <span className={SIDEBAR_LABEL}>{`+${more} more`}</span>
        </button>
      ) : null}
    </div>
  );
}

interface TopicsNavProps {
  collapsed: boolean;
}

function TopicsRow({ collapsed, active }: TopicsNavProps & { active: boolean }) {
  const router = useRouter();
  return (
    <NavRow
      slot="topics"
      icon={FolderKanban}
      label="Topics"
      collapsed={collapsed}
      active={active}
      onClick={() => router.push('/topics')}
    />
  );
}

function TopicsNavInner({ collapsed }: TopicsNavProps) {
  const pathname = usePathname();
  const id = useSearchParams()?.get('id') ?? null;
  const { data } = useTopics('open');
  const activeId = pathname?.startsWith('/topics/topic') ? id : null;
  const listedActive =
    !collapsed && activeId !== null && listed(data).shown.some((t) => t.id === activeId);
  return (
    <>
      <TopicsRow collapsed={collapsed} active={isTopicsPath(pathname) && !listedActive} />
      <TopicsNavList collapsed={collapsed} topics={data} activeId={activeId} />
    </>
  );
}

/** The Topics row and its list. Suspense because the open topic comes from the query string. */
export function TopicsNav({ collapsed }: TopicsNavProps) {
  const pathname = usePathname();
  return (
    <Suspense fallback={<TopicsRow collapsed={collapsed} active={isTopicsPath(pathname)} />}>
      <TopicsNavInner collapsed={collapsed} />
    </Suspense>
  );
}
