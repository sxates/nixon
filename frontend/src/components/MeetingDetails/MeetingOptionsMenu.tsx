'use client';

import { Pencil, Radio, Trash2 } from '@/components/ui/icons';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu';
import { MenuButton } from '@/components/ui/menu-button';

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

/** The unobtrusive "…" overflow menu in the meeting page header's actions. */
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
        <MenuButton label="Meeting options" />
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end">
        {canContinueRecording && (
          <DropdownMenuItem icon={Radio} onSelect={onContinueRecording}>
            Continue recording
          </DropdownMenuItem>
        )}
        {canEditManualMeeting && (
          <DropdownMenuItem icon={Pencil} onSelect={onEditManualMeeting}>
            Edit date & time
          </DropdownMenuItem>
        )}
        <DropdownMenuItem icon={Trash2} variant="destructive" onSelect={onDelete}>
          Delete meeting
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
