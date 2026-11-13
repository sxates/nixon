'use client';

/**
 * New topic (specs/0089): a name, an optional description, then straight to the topic page.
 * With `meetingId` (from a meeting page) it also asks whether the topic follows just that
 * meeting or its whole series, and whether to backfill the series' last 90 days.
 */

import { useEffect, useState } from 'react';
import { useRouter } from 'next/navigation';
import { toast } from 'sonner';
import { Button } from '@/components/ui/button';
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog';
import { Input } from '@/components/ui/input';
import { Label } from '@/components/ui/label';
import { Textarea } from '@/components/ui/textarea';
import { createTopic } from '@/lib/topics';
import { TopicScopeChoice, type TopicScope } from './TopicScopeChoice';

export interface NewTopicDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** A meeting to link the new topic to; adds the This meeting / series choice. */
  meetingId?: string;
}

export function NewTopicDialog({ open, onOpenChange, meetingId }: NewTopicDialogProps) {
  const router = useRouter();
  const [name, setName] = useState('');
  const [description, setDescription] = useState('');
  const [scope, setScope] = useState<TopicScope>('series');
  const [backfill, setBackfill] = useState(true);
  const [busy, setBusy] = useState(false);

  // Each opening starts fresh.
  useEffect(() => {
    if (!open) return;
    setName('');
    setDescription('');
    setScope('series');
    setBackfill(true);
    setBusy(false);
  }, [open]);

  const canCreate = name.trim().length > 0 && !busy;
  const series = !!meetingId && scope === 'series';

  const submit = async () => {
    if (!canCreate) return;
    setBusy(true);
    try {
      const topic = await createTopic({
        name: name.trim(),
        description: description.trim() || undefined,
        linkMeetingId: meetingId,
        linkSeries: series,
        backfill: series && backfill,
      });
      onOpenChange(false);
      router.push(`/topics/topic?id=${encodeURIComponent(topic.id)}`);
    } catch (err) {
      setBusy(false);
      toast.error('Could not create the topic', {
        description: err instanceof Error ? err.message : String(err),
      });
    }
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-[480px]">
        <DialogHeader>
          <DialogTitle>New topic</DialogTitle>
        </DialogHeader>
        <form
          className="flex flex-col gap-3"
          onSubmit={(e) => {
            e.preventDefault();
            void submit();
          }}
        >
          <div className="flex flex-col gap-1.5">
            <Label htmlFor="new-topic-name">Name</Label>
            <Input
              id="new-topic-name"
              value={name}
              onChange={(e) => setName(e.target.value)}
              autoFocus
            />
          </div>
          <div className="flex flex-col gap-1.5">
            <Label htmlFor="new-topic-description">Description</Label>
            <Textarea
              id="new-topic-description"
              value={description}
              onChange={(e) => setDescription(e.target.value)}
              rows={3}
            />
          </div>
          {meetingId ? (
            <TopicScopeChoice
              scope={scope}
              onScopeChange={setScope}
              backfill={backfill}
              onBackfillChange={setBackfill}
            />
          ) : null}
          <DialogFooter>
            <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
              Cancel
            </Button>
            <Button type="submit" disabled={!canCreate}>
              Create
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
