'use client';

/**
 * The Topic page body (specs/0089 §UI, the approved layout), top to bottom: the pinned
 * title row, then a scrolling body of the links + Description, My notes and My open action
 * items side by side (the to-dos use `TodoList`'s compact two-line rows to fit the half
 * column), the Suggested notes strip, and the timeline. Data comes from `useTopic`, which
 * refetches on `topics-updated`.
 */

import { Button } from '@/components/ui/button';
import { Callout } from '@/components/ui/callout';
import { EmptyState, PageLoading } from '@/components/ui/empty-state';
import { PageHeader } from '@/components/ui/page-header';
import { useBack } from '@/hooks/useBack';
import { useTopic } from '@/hooks/useTopics';
import { PAGE_CONTENT_WIDTH, PAGE_GUTTER } from '@/lib/page-width';
import { cn } from '@/lib/utils';
import { SuggestionStrip } from './SuggestionStrip';
import { Timeline } from './Timeline';
import { TopicHeader, TopicMeta } from './TopicHeader';
import { TopicNotesCard } from './TopicNotesCard';
import { TopicTodosCard } from './TopicTodosCard';

export function TopicView({ id }: { id: string }) {
  const goBack = useBack('/topics');
  const { data, loading, error, refresh } = useTopic(id);
  const back = { label: 'Back', onClick: goBack };

  if (loading && !data) return <PageLoading className="bg-background" />;
  if (!data) {
    const missing = !error || /not found|no longer exists/i.test(error);
    return (
      <div className="flex h-page flex-col bg-background">
        <PageHeader width="page" title="Topic" back={back} />
        {missing ? (
          <EmptyState
            title="This Topic no longer exists"
            action={
              <Button variant="outline" size="sm" onClick={back.onClick}>
                Back
              </Button>
            }
          />
        ) : (
          <div className={PAGE_GUTTER}>
            <div className={cn('mx-auto w-full', PAGE_CONTENT_WIDTH)}>
              <Callout tone="error">{error}</Callout>
            </div>
          </div>
        )}
      </div>
    );
  }

  return (
    <div className="flex h-page animate-fade-in flex-col bg-background">
      <TopicHeader detail={data} onChanged={refresh} />
      {/* pt-1: the scroll box clips overflow, and a chip's focus ring sits outside it. */}
      <div data-testid="topic-scroll" className={cn('flex-1 overflow-y-auto pb-12 pt-1', PAGE_GUTTER)}>
        <div className={cn('mx-auto flex w-full flex-col gap-3', PAGE_CONTENT_WIDTH)}>
          <TopicMeta detail={data} onChanged={refresh} />
          <div className="mt-3 grid grid-cols-1 gap-3 min-[900px]:grid-cols-2">
            <TopicNotesCard key={data.topic.id} topic={data.topic} />
            <TopicTodosCard
              topicId={data.topic.id}
              todos={data.todos}
              onChanged={refresh}
              hideFromTodo={data.topic.hideFromTodo}
            />
          </div>
          <SuggestionStrip suggestions={data.suggestions} onChanged={refresh} />
          <Timeline topicId={data.topic.id} entries={data.entries} onChanged={refresh} />
        </div>
      </div>
    </div>
  );
}
