'use client';

/**
 * specs/0019 WS2.3 (note 8) — per-segment speaker correction ("split").
 *
 * A subtle per-line control to move THIS transcript line to a different speaker when
 * diarization lumped it under the wrong one. Distinct from the inline name picker
 * (WS2.1), which reassigns the whole speaker's identity — this moves a single line.
 * The correction is sticky: the backend records it keyed by the line's transcript id
 * so it survives a later re-diarization (see api_set_segment_speaker).
 */

import { Check } from '@/components/ui/icons';
import { DeckIcon } from '@/components/ui/deck-icon';
import { MenuButton } from '@/components/ui/menu-button';
import { StatusDot } from '@/components/ui/status-dot';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu';
import { cn } from '@/lib/utils';
import { speakerBgClass } from '@/lib/speaker-colors';

export interface SegmentSpeakerMenuProps {
  transcriptId: string;
  /** The line's current speaker key (the one option shown as already-selected). */
  currentSpeakerKey: string;
  speakers: { speakerKey: string; displayName: string }[];
  /** Resolves success (specs/0041 WS7.2 — the caller owns the optimistic overlay and
   *  its revert-on-failure; this menu just fires the action). */
  onReassign: (transcriptId: string, speakerKey: string) => Promise<boolean>;
}

export function SegmentSpeakerMenu({
  transcriptId,
  currentSpeakerKey,
  speakers,
  onReassign,
}: SegmentSpeakerMenuProps) {
  // Nothing to move it to (only one speaker, or none) → no control.
  if (speakers.length < 2) return null;

  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        {/* The line's overflow `…` (design system: one MenuButton for overflow). 20px, not
            28: it sits inside a 12px typed name line. Hidden until the row is hovered
            (group/segment), so it doesn't clutter. Keyboard focus and an open menu also
            show it — but not plain `focus:`, which a mouse click leaves behind, keeping the
            icon lit after the pointer has gone. */}
        <MenuButton
          label="Move this line to another speaker"
          className="ml-0.5 h-5 w-5 opacity-0 transition-opacity focus-visible:opacity-100 data-[state=open]:opacity-100 group-hover/segment:opacity-100"
        />
      </DropdownMenuTrigger>
      <DropdownMenuContent align="start" className="w-48">
        <DropdownMenuLabel className="text-xs font-normal text-muted-foreground">
          This line is…
        </DropdownMenuLabel>
        <DropdownMenuSeparator />
        {speakers.map((s) => (
          <DropdownMenuItem
            key={s.speakerKey}
            onSelect={() => {
              if (s.speakerKey !== currentSpeakerKey) {
                void onReassign(transcriptId, s.speakerKey);
              }
            }}
            className="text-sm"
          >
            <StatusDot className={cn('mr-2', speakerBgClass(s.speakerKey))} />
            <span className="flex-1 truncate">{s.displayName}</span>
            {s.speakerKey === currentSpeakerKey && (
              <DeckIcon icon={Check} size={12} className="ml-1 flex-none text-muted-foreground" />
            )}
          </DropdownMenuItem>
        ))}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
