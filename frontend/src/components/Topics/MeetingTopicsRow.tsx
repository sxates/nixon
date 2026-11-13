'use client';

/**
 * Topics row on the meeting page (specs/0089): a chip per topic this meeting is on (each
 * opens the topic page), then Add to topic…. Refreshes on `topics-updated`.
 */

import { useCallback, useEffect, useRef, useState } from 'react';
import { useRouter } from 'next/navigation';
import { Button } from '@/components/ui/button';
import { safeListen } from '@/lib/safe-listen';
import { topicsForMeeting } from '@/lib/topics';
import type { Topic } from '@/types';
import { AddToTopicDialog } from './AddToTopicDialog';

export interface MeetingTopicsRowProps {
  meetingId: string;
  title: string | null | undefined;
  seriesKey: string | null | undefined;
}

export function MeetingTopicsRow({ meetingId, title, seriesKey }: MeetingTopicsRowProps) {
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
    <div className="mt-3 flex flex-wrap items-center gap-2">
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
      <Button variant="ghost" size="xs" onClick={() => setAdding(true)}>
        Add to topic…
      </Button>
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
