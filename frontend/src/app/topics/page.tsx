'use client';

import { useEffect, useMemo, useState } from 'react';
import { useRouter } from 'next/navigation';
import { toast } from 'sonner';
import { Button } from '@/components/ui/button';
import { Callout } from '@/components/ui/callout';
import { DeckIcon } from '@/components/ui/deck-icon';
import { EmptyState, PageLoading } from '@/components/ui/empty-state';
import { ChevronDown, ChevronRight, Plus } from '@/components/ui/icons';
import { PageHeader } from '@/components/ui/page-header';
import { NewTopicDialog } from '@/components/Topics/NewTopicDialog';
import { TopicIndexRows } from '@/components/Topics/TopicIndexRows';
import { useTopics } from '@/hooks/useTopics';
import { placeInSlots, sameOrder, sortTopics } from '@/lib/topic-order';
import { reorderTopics, updateTopic } from '@/lib/topics';
import { PAGE_CONTENT_WIDTH, PAGE_GUTTER } from '@/lib/page-width';
import { cn } from '@/lib/utils';
import type { TopicSummary } from '@/types';

export default function TopicsPage() {
  const router = useRouter();
  const { data, loading, error } = useTopics();
  const [creating, setCreating] = useState(false);
  const [showClosed, setShowClosed] = useState(false);
  // A drag or a star lands at once as a local override; the `topics-updated` refetch
  // replaces it with the saved state.
  const [override, setOverride] = useState<TopicSummary[] | null>(null);
  useEffect(() => setOverride(null), [data]);
  const topics = useMemo(() => override ?? (Array.isArray(data) ? data : []), [override, data]);

  // One manual order (specs/0090) shared by both sections.
  const { all, open, closed } = useMemo(() => {
    const sorted = sortTopics(topics);
    return {
      all: sorted,
      open: sorted.filter((t) => t.status === 'open'),
      closed: sorted.filter((t) => t.status === 'closed'),
    };
  }, [topics]);

  // A section reorders within the places it holds in the shared order, then the whole
  // order is saved.
  const reorderSection = (section: TopicSummary[], next: string[]) => {
    if (sameOrder(next, section.map((t) => t.id))) return;
    const ids = placeInSlots(all.map((t) => t.id), next);
    const rank = new Map(ids.map((id, i) => [id, i]));
    setOverride(topics.map((t) => ({ ...t, sortOrder: rank.get(t.id) ?? t.sortOrder })));
    reorderTopics(ids).catch((e: unknown) => {
      console.error('Failed to reorder topics:', e);
      setOverride(null);
      toast.error('Could not save the new order', {
        description: e instanceof Error ? e.message : String(e),
      });
    });
  };

  const toggleStar = (topic: TopicSummary) => {
    const starred = !topic.starred;
    setOverride(topics.map((t) => (t.id === topic.id ? { ...t, starred } : t)));
    updateTopic(topic.id, { starred }).catch((e: unknown) => {
      console.error('Failed to star topic:', e);
      setOverride(null);
      toast.error(starred ? 'Could not star the Topic' : 'Could not unstar the Topic', {
        description: e instanceof Error ? e.message : String(e),
      });
    });
  };

  const openTopic = (id: string) => router.push(`/topics/topic?id=${encodeURIComponent(id)}`);
  const newButton = (
    <Button variant="brand" onClick={() => setCreating(true)}>
      <Plus aria-hidden="true" />
      New Topic
    </Button>
  );

  return (
    <div className="h-page overflow-y-auto bg-background">
      <PageHeader
        width="page"
        title="Topics"
        subtitle="Ongoing subjects that span Meetings"
        actions={newButton}
      />
      {/* Gutter outside the column, as PageHeader does, so rows span the header's width. */}
      <div className={cn('pb-10', PAGE_GUTTER)}>
        <div className={cn('mx-auto w-full', PAGE_CONTENT_WIDTH)}>
          {loading ? (
            <PageLoading fill="pane" />
          ) : error ? (
            <Callout tone="error">{error}</Callout>
          ) : open.length === 0 && closed.length === 0 ? (
            // The header already carries New topic; one call to action is enough.
            <EmptyState
              title="No Topics yet"
              body="A Topic keeps a running log for one project across its Meetings."
            />
          ) : (
            <>
              <TopicIndexRows
                topics={open}
                onOpen={openTopic}
                onReorder={(next) => reorderSection(open, next)}
                onToggleStar={toggleStar}
              />
              {closed.length > 0 ? (
                <section className="mt-6">
                  <Button
                    variant="ghost"
                    size="xs"
                    aria-expanded={showClosed}
                    onClick={() => setShowClosed((v) => !v)}
                  >
                    <DeckIcon icon={showClosed ? ChevronDown : ChevronRight} size={14} />
                    {`Archived Topics (${closed.length})`}
                  </Button>
                  {showClosed ? (
                    <TopicIndexRows
                      topics={closed}
                      onOpen={openTopic}
                      onReorder={(next) => reorderSection(closed, next)}
                      onToggleStar={toggleStar}
                    />
                  ) : null}
                </section>
              ) : null}
            </>
          )}
        </div>
      </div>
      <NewTopicDialog open={creating} onOpenChange={setCreating} />
    </div>
  );
}
