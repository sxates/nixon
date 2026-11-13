'use client';

/**
 * The topic's link chips (specs/0089 §UI): each linked series or meeting, with an unlink ✕
 * that confirms first; Link a series…, which searches recorded meetings and links the
 * picked one's whole series (optionally backfilling 90 days); and the meeting count.
 */

import { useState } from 'react';
import { toast } from 'sonner';
import { LinkMeetingPicker } from '@/components/MeetingDetails/LinkMeetingPicker';
import { Button } from '@/components/ui/button';
import { Checkbox } from '@/components/ui/checkbox';
import { ConfirmDialog } from '@/components/ui/confirm-dialog';
import { DeckIcon } from '@/components/ui/deck-icon';
import { IconButton } from '@/components/ui/icon-button';
import { X } from '@/components/ui/icons';
import { linkTopic, unlinkTopic } from '@/lib/topics';
import type { TopicLink } from '@/types';

const errorText = (e: unknown) => (e instanceof Error ? e.message : String(e));

export interface TopicLinksProps {
  topicId: string;
  links: TopicLink[];
  meetingCount: number;
  onChanged?: () => void;
}

export function TopicLinks({ topicId, links, meetingCount, onChanged }: TopicLinksProps) {
  const [unlinking, setUnlinking] = useState<TopicLink | null>(null);
  const [busy, setBusy] = useState(false);
  const [picking, setPicking] = useState(false);
  const [backfill, setBackfill] = useState(true);

  const confirmUnlink = async () => {
    if (!unlinking) return;
    setBusy(true);
    try {
      await unlinkTopic(topicId, unlinking.kind, unlinking.ref);
      setUnlinking(null);
      onChanged?.();
    } catch (e) {
      console.error('Failed to unlink:', e);
      toast.error('Could not unlink that', { description: errorText(e) });
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="flex flex-wrap items-center gap-2">
      {links.map((l) => (
        <span
          key={`${l.kind}:${l.ref}`}
          className="inline-flex h-7 items-center gap-0.5 rounded-sm border border-border bg-card pl-2 text-caption font-semibold text-foreground"
        >
          {l.label}
          <IconButton
            size="sm"
            label={`Unlink ${l.label}`}
            icon={<DeckIcon icon={X} size={12} />}
            className="h-6 w-6"
            onClick={() => setUnlinking(l)}
          />
        </span>
      ))}
      <Button
        size="xs"
        variant="dashed"
        onClick={() => {
          setBackfill(true);
          setPicking(true);
        }}
      >
        Link a series…
      </Button>
      <span className="u-row-meta">{`${meetingCount} ${meetingCount === 1 ? 'meeting' : 'meetings'}`}</span>

      <ConfirmDialog
        open={unlinking !== null}
        onOpenChange={(open) => !open && setUnlinking(null)}
        title={unlinking ? `Unlink ${unlinking.label}?` : ''}
        body="Its meetings stop adding to this topic. Entries already on the timeline stay."
        confirmLabel="Unlink"
        busy={busy}
        onConfirm={confirmUnlink}
      />

      <LinkMeetingPicker
        open={picking}
        onOpenChange={setPicking}
        title="Link a series"
        description="Pick any meeting from the series. Every meeting in it feeds this topic."
        excludeIds={[]}
        onPick={async (meetingId) => {
          await linkTopic(topicId, meetingId, 'series', backfill);
        }}
        onLink={() => onChanged?.()}
      >
        <Checkbox
          checked={backfill}
          onCheckedChange={setBackfill}
          label="Also add this series' meetings from the last 90 days"
        />
      </LinkMeetingPicker>
    </div>
  );
}
