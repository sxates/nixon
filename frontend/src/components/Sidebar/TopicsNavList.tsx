'use client';

/**
 * The Topics nav entry (specs/0089): the Topics row, then the starred open topics under it
 * in the Topics page's manual order (specs/0090, owner decision 2026-11-16), each with its
 * pending-suggestion count. With none starred, just the Topics row. The list shows only in
 * the expanded sidebar; the collapsed rail has no room for names.
 *
 * On a topic's page, that topic's row is the lit one (index bar, full-strength text) and the
 * Topics row stays dark, so one row is lit at a time. When the open topic isn't listed
 * (archived, not starred, or the rail is collapsed) the Topics row lights instead.
 */

import React, { Suspense, useMemo } from 'react';
import { usePathname, useRouter, useSearchParams } from 'next/navigation';
import { Badge } from '@/components/ui/badge';
import { FolderKanban } from '@/components/ui/icons';
import { useTopics } from '@/hooks/useTopics';
import { sidebarTopics } from '@/lib/topic-order';
import { pendingLabel } from '@/lib/topics';
import type { TopicSummary } from '@/types';
import { IndexBar, NavRow } from './SidebarRow';
import { SIDEBAR_ICON_SLOT, SIDEBAR_LABEL, SIDEBAR_ROW } from './row';
import { cn } from '@/lib/utils';

const ROW_CLASS = cn(
  SIDEBAR_ROW,
  'relative h-8 transition-colors hover:bg-key focus:outline-none focus-visible:ring-2 focus-visible:ring-ring',
);

const isTopicsPath = (p: string | null) => p?.startsWith('/topics') ?? false;

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
  const shown = useMemo(() => sidebarTopics(topics), [topics]);
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
    !collapsed && activeId !== null && sidebarTopics(data).some((t) => t.id === activeId);
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
