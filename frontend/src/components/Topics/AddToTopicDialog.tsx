'use client';

/**
 * Add to topic… (specs/0089): pick an open topic, choose This meeting or the whole series
 * (with the 90-day backfill for a series), or start a New topic from this meeting.
 */

import { useEffect, useState } from 'react';
import { toast } from 'sonner';
import { Button } from '@/components/ui/button';
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog';
import { ListRow } from '@/components/ui/list-row';
import { linkTopic, listTopics } from '@/lib/topics';
import type { TopicSummary } from '@/types';
import { NewTopicDialog } from './NewTopicDialog';
import { TopicScopeChoice, type TopicScope } from './TopicScopeChoice';

export interface AddToTopicDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  meetingId: string;
  /** Topics this meeting is already on; hidden from the list. */
  linkedIds: string[];
  /** Series option needs a series key or a title to match on. */
  seriesAvailable: boolean;
}

export function AddToTopicDialog({
  open,
  onOpenChange,
  meetingId,
  linkedIds,
  seriesAvailable,
}: AddToTopicDialogProps) {
  const [topics, setTopics] = useState<TopicSummary[]>([]);
  const [picked, setPicked] = useState<string | null>(null);
  const [scope, setScope] = useState<TopicScope>('meeting');
  const [backfill, setBackfill] = useState(false);
  const [busy, setBusy] = useState(false);
  const [creating, setCreating] = useState(false);

  useEffect(() => {
    if (!open) return;
    setPicked(null);
    setScope('meeting');
    setBackfill(false);
    setBusy(false);
    let live = true;
    listTopics('open')
      .then((t) => live && setTopics(t))
      .catch(() => live && toast.error('Could not load topics'));
    return () => {
      live = false;
    };
  }, [open]);

  const choices = topics.filter((t) => !linkedIds.includes(t.id));
  const series = scope === 'series';

  const submit = async () => {
    if (!picked || busy) return;
    setBusy(true);
    try {
      await linkTopic(picked, meetingId, scope, series && backfill);
      onOpenChange(false);
    } catch (err) {
      setBusy(false);
      toast.error('Could not add to the topic', {
        description: err instanceof Error ? err.message : String(err),
      });
    }
  };

  return (
    <>
      <Dialog open={open && !creating} onOpenChange={onOpenChange}>
        <DialogContent className="sm:max-w-[480px]">
          <DialogHeader>
            <DialogTitle>Add to topic…</DialogTitle>
          </DialogHeader>
          <div className="flex max-h-64 flex-col overflow-y-auto">
            {choices.map((t) => (
              <ListRow
                key={t.id}
                title={t.name}
                selected={picked === t.id}
                onClick={() => setPicked(t.id)}
              />
            ))}
          </div>
          <TopicScopeChoice
            scope={scope}
            onScopeChange={setScope}
            backfill={backfill}
            onBackfillChange={setBackfill}
            seriesAvailable={seriesAvailable}
          />
          <DialogFooter>
            <Button type="button" variant="outline" onClick={() => setCreating(true)}>
              New topic
            </Button>
            <Button type="button" disabled={!picked || busy} onClick={() => void submit()}>
              Add
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
      <NewTopicDialog
        open={open && creating}
        onOpenChange={(o) => {
          setCreating(o);
          if (!o) onOpenChange(false);
        }}
        meetingId={meetingId}
        seriesAvailable={seriesAvailable}
      />
    </>
  );
}
