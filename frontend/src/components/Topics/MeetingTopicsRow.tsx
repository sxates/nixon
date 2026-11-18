'use client';

/**
 * Topics row on the meeting page (specs/0089): a chip per topic this meeting is on (each
 * opens the topic page), then Add to topic…. Refreshes on `topics-updated`.
 */

import { useCallback, useEffect, useRef, useState } from 'react';
import { useRouter } from 'next/navigation';
import { Button } from '@/components/ui/button';
import { Plus } from '@/components/ui/icons';
import { safeListen } from '@/lib/safe-listen';
import { cn } from '@/lib/utils';
import { topicsForMeeting } from '@/lib/topics';
import type { Topic } from '@/types';
import { AddToTopicDialog } from './AddToTopicDialog';

export interface MeetingTopicsRowProps {
  meetingId: string;
  title: string | null | undefined;
  seriesKey: string | null | undefined;
  /** Replaces the row's default top margin (the record screen's strip sets its own). */
  className?: string;
  /**
   * `row` (default, the meeting page): a "Topics" label, chips, then a dashed "Add to Topic…".
   * `strip` (the record screen's meeting bar): no label, one line that never wraps, the
   * chips and an "Add Topic" key all in the bar's key style (`keyClassName`).
   */
  variant?: 'row' | 'strip';
  /** Extra classes on every key in the `strip` variant. */
  keyClassName?: string;
}

export function MeetingTopicsRow({
  meetingId,
  title,
  seriesKey,
  className = 'mt-3',
  variant = 'row',
  keyClassName,
}: MeetingTopicsRowProps) {
  const strip = variant === 'strip';
  const router = useRouter();
  const [topics, setTopics] = useState<Topic[]>([]);
  const [adding, setAdding] = useState(false);
  const seq = useRef(0);

  const load = useCallback(async () => {
    const mine = ++seq.current;
    try {
      const next = await topicsForMeeting(meetingId);
      if (mine === seq.current) setTopics(Array.isArray(next) ? next : []);
    } catch (e) {
      console.error('Could not load this meeting’s topics', e);
    }
  }, [meetingId]);

  useEffect(() => {
    void load();
  }, [load]);
  useEffect(() => safeListen('topics-updated', () => void load()), [load]);

  const seriesAvailable = !!seriesKey || !!title?.trim();

  return (
    <div className={cn('flex items-center gap-2', strip ? 'min-w-0 flex-nowrap' : 'flex-wrap', className)}>
      {!strip && <span className="text-xs font-semibold text-muted-foreground">Topics</span>}
      {topics.map((t) => (
        <Button
          key={t.id}
          variant="outline"
          size="xs"
          title={strip ? t.name : undefined}
          className={strip ? cn('min-w-0 max-w-[160px]', keyClassName) : undefined}
          onClick={() => router.push(`/topics/topic?id=${encodeURIComponent(t.id)}`)}
        >
          {strip ? <span className="truncate">{t.name}</span> : t.name}
        </Button>
      ))}
      {strip ? (
        <Button variant="outline" size="xs" className={cn('flex-shrink-0', keyClassName)} onClick={() => setAdding(true)}>
          Add Topic
        </Button>
      ) : (
        <Button variant="dashed" size="xs" onClick={() => setAdding(true)}>
          <Plus />
          Add to Topic…
        </Button>
      )}
      <AddToTopicDialog
        open={adding}
        onOpenChange={setAdding}
        meetingId={meetingId}
        linkedIds={topics.map((t) => t.id)}
        seriesAvailable={seriesAvailable}
      />
    </div>
  );
}
