'use client';

/**
 * Topics row on the meeting page (specs/0089): a chip per topic this meeting is on (each
 * opens the topic page), then Add to topic…. Refreshes on `topics-updated`.
 */

import { useCallback, useEffect, useRef, useState, type ReactNode } from 'react';
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
  /** Extra controls at the end of the row (the record screen's Add To-Do). */
  trailing?: ReactNode;
}

export function MeetingTopicsRow({ meetingId, title, seriesKey, className = 'mt-3', trailing }: MeetingTopicsRowProps) {
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
    <div className={cn('flex flex-wrap items-center gap-2', className)}>
      <span className="text-xs font-semibold text-muted-foreground">Topics</span>
      {topics.map((t) => (
        <Button
          key={t.id}
          variant="outline"
          size="xs"
          onClick={() => router.push(`/topics/topic?id=${encodeURIComponent(t.id)}`)}
        >
          {t.name}
        </Button>
      ))}
      <Button variant="dashed" size="xs" onClick={() => setAdding(true)}>
        <Plus />
        Add to Topic…
      </Button>
      <AddToTopicDialog
        open={adding}
        onOpenChange={setAdding}
        meetingId={meetingId}
        linkedIds={topics.map((t) => t.id)}
        seriesAvailable={seriesAvailable}
      />
      {trailing}
    </div>
  );
}
