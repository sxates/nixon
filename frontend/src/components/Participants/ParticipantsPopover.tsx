'use client';

/**
 * ParticipantsPopover (specs/0017, Phase C) — a lightweight trigger that surfaces the
 * compact ParticipantsPanel for the in-progress meeting. Used in the record-screen header
 * (and reachable from recording chrome) so the roster can be viewed/curated live without
 * leaving the recording. Add/remove are plain DB writes that work mid-recording.
 *
 * The trigger is token-only (specs/0057), so it reads correctly on every surface and in
 * both themes — no per-placement `tone` variant.
 */

import { Users } from 'lucide-react';
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from '@/components/ui/popover';
import { cn } from '@/lib/utils';
import { ParticipantsPanel } from './ParticipantsPanel';

interface ParticipantsPopoverProps {
  meetingId: string;
  className?: string;
}

export function ParticipantsPopover({
  meetingId,
  className,
}: ParticipantsPopoverProps) {
  return (
    <Popover>
      <PopoverTrigger asChild>
        <button
          type="button"
          aria-label="Participants"
          title="Participants"
          className={cn(
            // h-8 to match the mode chip and template picker beside it on the record
            // header (owner feedback 2026-09-21).
            'inline-flex h-8 items-center gap-1.5 rounded-lg px-3 text-xs font-semibold transition-colors focus:outline-none focus-visible:ring-2',
            'border border-border bg-card text-foreground hover:bg-muted focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-card',
            className,
          )}
        >
          <Users size={14} />
          Participants
        </button>
      </PopoverTrigger>
      <PopoverContent align="end" className="w-80 p-3">
        <ParticipantsPanel meetingId={meetingId} variant="compact" />
      </PopoverContent>
    </Popover>
  );
}
