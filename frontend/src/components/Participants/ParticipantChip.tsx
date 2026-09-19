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

import { MoreHorizontal, UserCheck, X } from 'lucide-react';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu';
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from '@/components/ui/tooltip';
import { speakerBgClass } from '@/lib/speaker-colors';
import { cn } from '@/lib/utils';
import type { MeetingParticipant } from '@/types';

/** Initials for the avatar dot, e.g. "Priya Sharma" → "PS". */
function initials(name: string): string {
  const parts = name.trim().split(/\s+/).filter(Boolean);
  if (parts.length === 0) return '?';
  if (parts.length === 1) return parts[0]!.charAt(0).toUpperCase();
  return (parts[0]!.charAt(0) + parts[parts.length - 1]!.charAt(0)).toUpperCase();
}

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
  // Render the cached directory photo (specs/0038 WS3) in place of initials when
  // present + loadable; a broken image (onError) or absent photo falls back to the
  // colored initials chip — mirroring AvatarStack for visual consistency.
  const [photoFailed, setPhotoFailed] = useState(false);
  const showPhoto = !!participant.photoDataUri && !photoFailed;
  return (
    <TooltipProvider delayDuration={400}>
      {/* 0.1.0 canvas feedback: unboxed — avatar + name as plain text; the secondary actions
          (more / remove) appear on hover or keyboard focus only. */}
      <div className="group inline-flex items-center gap-1.5 text-xs">
        {showPhoto ? (
          // A self-contained base64 `data:` URI — no image server in the Tauri shell,
          // so a plain <img> (not next/image) is correct here.
          // eslint-disable-next-line @next/next/no-img-element
          <img
            src={participant.photoDataUri as string}
            alt=""
            aria-hidden="true"
            onError={() => setPhotoFailed(true)}
            className="h-5 w-5 flex-shrink-0 rounded-full object-cover"
          />
        ) : (
          <span
            className={cn(
              'flex h-5 w-5 flex-shrink-0 items-center justify-center rounded-full text-[9px] font-semibold',
              // `bg-muted` is the null-key fallback and is a light chip — its ink is the
              // foreground, not the background.
              speakerBgClass(colorKey) === 'bg-muted' ? 'text-foreground' : 'text-background',
              speakerBgClass(colorKey),
            )}
            aria-hidden="true"
          >
            {initials(name)}
          </span>
        )}

        {/* Click the body to edit the person (name / role / notes). */}
        <Tooltip>
          <TooltipTrigger asChild>
            <button
              type="button"
              onClick={onEdit}
              className="flex items-center gap-1 rounded px-0.5 py-0.5 font-medium text-foreground hover:text-brand"
              title={`Edit ${name}`}
            >
              {/* specs/0019 WS3.1 (note 2): the call roster doesn't need titles —
                  name + "spoke" badge only. Role still shows in the edit dialog. */}
              <span className="max-w-[160px] truncate">{name}</span>
              {spoke && (
                <span className="rounded-[3px] bg-success/10 px-1.5 py-px text-[10px] font-medium text-success">
                  spoke
                </span>
              )}
            </button>
          </TooltipTrigger>
          <TooltipContent side="bottom">Edit this person</TooltipContent>
        </Tooltip>

        {/* Secondary actions: "This is me" lives in an overflow menu so the primary
            affordances stay edit (body) + remove (×). */}
        <DropdownMenu>
          <Tooltip>
            <TooltipTrigger asChild>
              <DropdownMenuTrigger asChild>
                <button
                  type="button"
                  aria-label={`More actions for ${name}`}
                  className="flex-shrink-0 rounded-[3px] p-0.5 text-muted-foreground opacity-0 transition-opacity hover:bg-muted hover:text-foreground focus-visible:opacity-100 group-hover:opacity-100 group-focus-within:opacity-100 data-[state=open]:opacity-100"
                >
                  <MoreHorizontal size={12} />
                </button>
              </DropdownMenuTrigger>
            </TooltipTrigger>
            <TooltipContent side="bottom">More actions</TooltipContent>
          </Tooltip>
          <DropdownMenuContent align="end">
            <DropdownMenuItem onSelect={onClaimAsMe}>
              <UserCheck size={14} className="mr-2" />
              This is me
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>

        <Tooltip>
          <TooltipTrigger asChild>
            <button
              type="button"
              onClick={onRemove}
              aria-label={`Remove ${name}`}
              className="flex-shrink-0 rounded-[3px] p-0.5 text-muted-foreground opacity-0 transition-opacity hover:bg-muted hover:text-foreground focus-visible:opacity-100 group-hover:opacity-100 group-focus-within:opacity-100"
            >
              <X size={12} />
            </button>
          </TooltipTrigger>
          <TooltipContent side="bottom">
            Remove from this meeting (stays in People)
          </TooltipContent>
        </Tooltip>
      </div>
    </TooltipProvider>
  );
}
