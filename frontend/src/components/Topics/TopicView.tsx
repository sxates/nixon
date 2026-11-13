'use client';

/**
 * The Topic page body (specs/0089 §UI, the approved layout), top to bottom: header, the
 * My notes / My open action items cards, the Suggested notes strip, and the timeline.
 * Data comes from `useTopic`, which refetches on `topics-updated`.
 */

import { useRouter } from 'next/navigation';
import { Button } from '@/components/ui/button';
import { Callout } from '@/components/ui/callout';
import { EmptyState, PageLoading } from '@/components/ui/empty-state';
import { PageHeader } from '@/components/ui/page-header';
import { useTopic } from '@/hooks/useTopics';
import { PAGE_CONTENT_WIDTH, PAGE_GUTTER } from '@/lib/page-width';
import { cn } from '@/lib/utils';
import { SuggestionStrip } from './SuggestionStrip';
import { Timeline } from './Timeline';
import { TopicHeader } from './TopicHeader';
import { TopicNotesCard } from './TopicNotesCard';
import { TopicTodosCard } from './TopicTodosCard';

export function TopicView({ id }: { id: string }) {
  const router = useRouter();
  const { data, loading, error, refresh } = useTopic(id);
  const back = { label: 'Topics', onClick: () => router.push('/topics') };

  if (loading && !data) return <PageLoading className="bg-background" />;
  if (!data) {
    const missing = !error || /not found|no longer exists/i.test(error);
    return (
      <div className="flex h-page flex-col bg-background">
        <PageHeader width="page" title="Topic" back={back} />
        {missing ? (
          <EmptyState
            title="This topic no longer exists"
            action={
              <Button variant="outline" size="sm" onClick={back.onClick}>
                Back to Topics
              </Button>
            }
          />
        ) : (
          <div className={cn('mx-auto w-full', PAGE_CONTENT_WIDTH, PAGE_GUTTER)}>
            <Callout tone="error">{error}</Callout>
          </div>
        )}
      </div>
    );
  }

  return (
    <div data-testid="topic-scroll" className="h-page animate-fade-in overflow-y-auto bg-background">
      <TopicHeader detail={data} onChanged={refresh} />
      <div className={cn('mx-auto flex w-full flex-col gap-3 pb-12 pt-6', PAGE_CONTENT_WIDTH, PAGE_GUTTER)}>
        <div className="grid grid-cols-1 gap-3 min-[900px]:grid-cols-2">
          <TopicNotesCard key={data.topic.id} topic={data.topic} />
          <TopicTodosCard topicId={data.topic.id} todos={data.todos} onChanged={refresh} />
        </div>
        <SuggestionStrip suggestions={data.suggestions} onChanged={refresh} />
        <Timeline topicId={data.topic.id} entries={data.entries} onChanged={refresh} />
      </div>
    </div>
  );
}
