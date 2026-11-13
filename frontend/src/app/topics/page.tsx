'use client';

import { useMemo, useState } from 'react';
import { useRouter } from 'next/navigation';
import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import { Callout } from '@/components/ui/callout';
import { DeckIcon } from '@/components/ui/deck-icon';
import { EmptyState, PageLoading } from '@/components/ui/empty-state';
import { ChevronDown, ChevronRight, Plus } from '@/components/ui/icons';
import { ListRow } from '@/components/ui/list-row';
import { PageHeader } from '@/components/ui/page-header';
import { NewTopicDialog } from '@/components/Topics/NewTopicDialog';
import { useTopics } from '@/hooks/useTopics';
import { formatDecisionDate } from '@/lib/topics';
import { PAGE_CONTENT_WIDTH, PAGE_GUTTER } from '@/lib/page-width';
import { cn } from '@/lib/utils';
import type { TopicSummary } from '@/types';

function TopicRows({ topics, onOpen }: { topics: TopicSummary[]; onOpen: (id: string) => void }) {
  return (
    <div className="flex flex-col">
      {topics.map((t) => {
        const when = t.lastActivity ? formatDecisionDate(t.lastActivity) : '';
        return (
          <ListRow
            key={t.id}
            ruled
            title={t.name}
            meta={[t.description.split('\n')[0], when && `Last activity ${when}`]
              .filter(Boolean)
              .join(' · ')}
            trailing={
              t.pendingCount > 0 ? (
                <Badge tone="brand" title={`${t.pendingCount} waiting`}>
                  {t.pendingCount}
                </Badge>
              ) : null
            }
            onClick={() => onOpen(t.id)}
          />
        );
      })}
    </div>
  );
}

export default function TopicsPage() {
  const router = useRouter();
  const { data, loading, error } = useTopics();
  const [creating, setCreating] = useState(false);
  const [showClosed, setShowClosed] = useState(false);

  const { open, closed } = useMemo(() => {
    const all = [...(data ?? [])].sort((a, b) =>
      (b.lastActivity ?? '').localeCompare(a.lastActivity ?? ''),
    );
    return {
      open: all.filter((t) => t.status === 'open'),
      closed: all.filter((t) => t.status === 'closed'),
    };
  }, [data]);

  const openTopic = (id: string) => router.push(`/topics/topic?id=${encodeURIComponent(id)}`);
  const newButton = (
    <Button onClick={() => setCreating(true)}>
      <DeckIcon icon={Plus} size={16} />
      New topic
    </Button>
  );

  return (
    <div className="h-page overflow-y-auto bg-background">
      <PageHeader
        width="page"
        title="Topics"
        subtitle="Ongoing subjects that span meetings"
        actions={newButton}
      />
      <div className={cn('mx-auto w-full pb-10', PAGE_CONTENT_WIDTH, PAGE_GUTTER)}>
        {loading ? (
          <PageLoading fill="pane" />
        ) : error ? (
          <Callout tone="error">{error}</Callout>
        ) : open.length === 0 && closed.length === 0 ? (
          <EmptyState title="No topics yet" action={newButton} />
        ) : (
          <>
            <TopicRows topics={open} onOpen={openTopic} />
            {closed.length > 0 ? (
              <section className="mt-6">
                <Button
                  variant="ghost"
                  size="xs"
                  aria-expanded={showClosed}
                  onClick={() => setShowClosed((v) => !v)}
                >
                  <DeckIcon icon={showClosed ? ChevronDown : ChevronRight} size={14} />
                  {`Closed topics (${closed.length})`}
                </Button>
                {showClosed ? <TopicRows topics={closed} onOpen={openTopic} /> : null}
              </section>
            ) : null}
          </>
        )}
      </div>
      <NewTopicDialog open={creating} onOpenChange={setCreating} />
    </div>
  );
}
