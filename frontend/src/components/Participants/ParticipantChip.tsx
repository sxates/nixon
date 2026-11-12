/**
 * One participant in the roster — avatar or initials, the name, and the hover actions
 * (edit, claim-as-me, remove). Extracted from `ParticipantsPanel` (specs/0064 W4), which was
 * 63 lines short of the 800-line cap before the grid work.
 *
 * The actions are deliberately hover-and-keyboard only: showing them always turns a readable
 * list of names into a row of buttons. They reserve their width inside the chip's own grid
 * cell, so revealing them cannot push the neighbouring names around (specs/0064 item 3).
 */
'use client';

import { UserCheck, X } from '@/components/ui/icons';
import { Avatar } from '@/components/ui/avatar';
import { Badge } from '@/components/ui/badge';
import { DeckIcon } from '@/components/ui/deck-icon';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu';
import { IconButton } from '@/components/ui/icon-button';
import { MenuButton } from '@/components/ui/menu-button';
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from '@/components/ui/tooltip';
import type { MeetingParticipant } from '@/types';

export interface ParticipantChipProps {
  participant: MeetingParticipant;
  spoke: boolean;
  onEdit: () => void;
  onRemove: () => void;
  onClaimAsMe: () => void;
}

export function ParticipantChip({
  participant,
  spoke,
  onEdit,
  onRemove,
  onClaimAsMe,
}: ParticipantChipProps) {
  const name = participant.displayName?.trim() || 'Unnamed';
  const colorKey = participant.email || name;
  return (
    <TooltipProvider delayDuration={400}>
      {/* 0.1.0 canvas feedback: unboxed — avatar + name as plain text; the secondary actions
          (more / remove) appear on hover or keyboard focus only.
          specs/0064 W4: visibility keys on hover, the button's OWN focus-visible, and an
          actually-open menu — never `group-focus-within`, which kept a chip lit after a
          click because the trigger holds focus long after the pointer has gone. */}
      <div className="group flex min-w-0 items-center gap-1.5 text-xs">
        {/* The cached directory photo (specs/0038 WS3) when present and loadable, else the
            coloured initials chip — the shared Avatar handles the onError fallback. */}
        <Avatar name={name} photo={participant.photoDataUri} colorKey={colorKey} size={22} />

        {/* Click the body to edit the person (name / role / notes). */}
        <Tooltip>
          <TooltipTrigger asChild>
            <button // ui-gate: allow unboxed name-as-text edit trigger (0.1.0 canvas feedback); Button chrome would box every roster name
              type="button"
              onClick={onEdit}
              className="flex items-center gap-1 rounded-md px-0.5 py-0.5 font-medium text-foreground hover:text-brand"
              title={`Edit ${name}`}
            >
              {/* specs/0019 WS3.1 (note 2): the call roster doesn't need titles —
                  name + "spoke" badge only. Role still shows in the edit dialog. */}
              <span className="max-w-[160px] truncate">{name}</span>
              {spoke && (
                <Badge tone="success" className="px-1.5 py-px font-medium">
                  spoke
                </Badge>
              )}
            </button>
          </TooltipTrigger>
          <TooltipContent side="bottom">Edit this person</TooltipContent>
        </Tooltip>

        {/* Secondary actions: "This is me" lives in an overflow menu so the primary
            affordances stay edit (body) + remove (×). */}
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <MenuButton
              label="More actions"
              aria-label={`More actions for ${name}`}
              side="bottom"
              className="opacity-0 transition-opacity focus-visible:opacity-100 group-hover:opacity-100 data-[state=open]:opacity-100"
            />
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end">
            <DropdownMenuItem icon={UserCheck} onSelect={onClaimAsMe}>
              This is me
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>

        <IconButton
          variant="ghost"
          side="bottom"
          label="Remove from this meeting (stays in People)"
          aria-label={`Remove ${name}`}
          onClick={onRemove}
          icon={<DeckIcon icon={X} size={12} />}
          className="opacity-0 transition-opacity focus-visible:opacity-100 group-hover:opacity-100"
        />
      </div>
    </TooltipProvider>
  );
}
