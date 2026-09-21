'use client';

import { useState } from 'react';
import { toast } from 'sonner';
import { Loader2 } from 'lucide-react';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog';
import { Button } from '@/components/ui/button';
import { deleteManualMeeting } from '@/lib/day-agenda';

interface DeleteManualMeetingDialogProps {
  /** Controlled open state. */
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** The manual entry's meeting id; `null` while no entry is targeted. */
  meetingId: string | null;
  /** Optional title for a friendlier confirmation + success toast. */
  meetingTitle?: string;
  /** Called after a successful delete (so the caller can refresh the agenda). */
  onDeleted: () => void | Promise<void>;
}

/**
 * Confirm-then-delete for a manually added meeting (specs/0069 W3). Mirrors
 * `DeleteMeetingDialog`'s confirm shape, but this is a DIFFERENT command
 * (`api_delete_manual_meeting` via `deleteManualMeeting`): a manual row is still a
 * bare `scheduled` prep row with no recording, transcript, or summary behind it —
 * deleting it is undoing something you typed, not destroying a recording. It's the
 * user's own row (not a calendar event), so the way to get rid of it is to delete
 * it, never to hide it.
 *
 * The backend refuses once the row has been recorded ("edit/delete it from the
 * meeting page instead") — that message is surfaced verbatim in the toast, same
 * as `ClearVoiceprintsDialog`, rather than swallowed behind a generic one.
 */
export function DeleteManualMeetingDialog({
  open,
  onOpenChange,
  meetingId,
  meetingTitle,
  onDeleted,
}: DeleteManualMeetingDialogProps) {
  const [isDeleting, setIsDeleting] = useState(false);
  const label = meetingTitle?.trim();

  const handleConfirm = async () => {
    if (isDeleting || !meetingId) return;
    setIsDeleting(true);
    try {
      await deleteManualMeeting(meetingId);
      toast.success(label ? `Deleted "${label}"` : 'Meeting deleted');
      onOpenChange(false);
      await onDeleted();
    } catch (error) {
      console.error('Failed to delete manual meeting:', error);
      toast.error('Could not delete this meeting', {
        description: error instanceof Error ? error.message : String(error),
      });
    } finally {
      setIsDeleting(false);
    }
  };

  return (
    <Dialog open={open} onOpenChange={(next) => (isDeleting ? undefined : onOpenChange(next))}>
      <DialogContent className="max-w-md">
        <DialogHeader>
          <DialogTitle>Delete meeting</DialogTitle>
          <DialogDescription>
            Delete {label ? `"${label}"` : 'this meeting'}? This can&apos;t be undone.
          </DialogDescription>
        </DialogHeader>
        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)} disabled={isDeleting}>
            Cancel
          </Button>
          <Button variant="destructive" onClick={handleConfirm} disabled={isDeleting}>
            {isDeleting && <Loader2 className="mr-2 h-4 w-4 animate-spin" />}
            Delete
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
