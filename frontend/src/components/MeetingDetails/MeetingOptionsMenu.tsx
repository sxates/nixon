'use client';

import { MoreHorizontal, Pencil, Radio, Trash2 } from 'lucide-react';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu';

interface MeetingOptionsMenuProps {
  /** Continue recording (specs/0037): resume capture INTO this same meeting. */
  canContinueRecording: boolean;
  onContinueRecording: () => void;
  /** specs/0069b review fix 2 — edit date/time/title/link for a manual entry that
   *  hasn't been recorded yet. Omitted (no menu item) once it's not editable. */
  canEditManualMeeting?: boolean;
  onEditManualMeeting?: () => void;
  onDelete: () => void;
}

/** The unobtrusive "…" overflow menu on the meeting-details identity row. */
export function MeetingOptionsMenu({
  canContinueRecording,
  onContinueRecording,
  canEditManualMeeting = false,
  onEditManualMeeting,
  onDelete,
}: MeetingOptionsMenuProps) {
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <button
          type="button"
          aria-label="Meeting options"
          title="Meeting options"
          className="mt-1 inline-flex h-7 w-7 flex-shrink-0 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground focus:outline-none focus-visible:ring-2 focus-visible:ring-ring"
        >
          <MoreHorizontal className="h-[18px] w-[18px]" />
        </button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end">
        {canContinueRecording && (
          <DropdownMenuItem onSelect={onContinueRecording}>
            <Radio className="mr-2 h-4 w-4" />
            Continue recording
          </DropdownMenuItem>
        )}
        {canEditManualMeeting && (
          <DropdownMenuItem onSelect={onEditManualMeeting}>
            <Pencil className="mr-2 h-4 w-4" />
            Edit date & time
          </DropdownMenuItem>
        )}
        <DropdownMenuItem
          onSelect={onDelete}
          className="text-destructive focus:text-destructive"
        >
          <Trash2 className="mr-2 h-4 w-4" />
          Delete meeting
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
