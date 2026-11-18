'use client';

/**
 * ParticipantsPopover (specs/0017, Phase C) — a lightweight trigger that surfaces the
 * compact ParticipantsPanel for the in-progress meeting. Used in the record screen's top
 * strips (RecordMeetingControls) so the roster can be viewed/curated live without
 * leaving the recording. Add/remove are plain DB writes that work mid-recording.
 *
 * The trigger is token-only (specs/0057), so it reads correctly on every surface and in
 * both themes — no per-placement `tone` variant.
 */

import { Button } from '@/components/ui/button';
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from '@/components/ui/popover';
import { ParticipantsPanel } from './ParticipantsPanel';

interface ParticipantsPopoverProps {
  meetingId: string;
  /** Trigger size: `sm` (h-8, default) or `xs` (h-7, the record screen's strip keys). */
  size?: 'sm' | 'xs';
  className?: string;
}

export function ParticipantsPopover({
  meetingId,
  size = 'sm',
  className,
}: ParticipantsPopoverProps) {
  return (
    <Popover>
      <PopoverTrigger asChild>
        <Button
          type="button"
          variant="outline"
          size={size}
          aria-label="Participants"
          title="Participants"
          className={className}
        >
          Participants
        </Button>
      </PopoverTrigger>
      <PopoverContent align="end" className="w-80 p-3">
        <ParticipantsPanel meetingId={meetingId} variant="compact" />
      </PopoverContent>
    </Popover>
  );
}
