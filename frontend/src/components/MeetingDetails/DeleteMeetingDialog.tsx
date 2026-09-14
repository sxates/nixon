'use client';

import { useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
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

interface DeleteMeetingDialogProps {
  /** Controlled open state. */
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** Meeting to delete. */
  meetingId: string;
  /** Optional title for a friendlier success toast. */
  meetingTitle?: string;
  /** Called after a successful delete (navigate away / refetch the list). */
  onDeleted: () => void | Promise<void>;
}

/**
 * Confirm-then-delete dialog for a meeting. Reuses the shared `Dialog` primitive;
 * the destructive action is wired to the existing `api_delete_meeting` command,
 * which cascades the DB rows (and, per spec 0015 Phase A, the on-disk recording).
 */
export function DeleteMeetingDialog({
  open,
  onOpenChange,
  meetingId,
  meetingTitle,
  onDeleted,
}: DeleteMeetingDialogProps) {
  const [isDeleting, setIsDeleting] = useState(false);

  const handleConfirm = async () => {
    if (isDeleting) return;
    setIsDeleting(true);
    try {
      await invoke('api_delete_meeting', { meetingId });
      const label = meetingTitle?.trim();
      toast.success(label ? `Deleted "${label}"` : 'Meeting deleted');
      onOpenChange(false);
      await onDeleted();
    } catch (error) {
      console.error('Failed to delete meeting:', error);
      toast.error('Could not delete this meeting. Please try again.');
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
            Delete this meeting? This permanently removes the recording, transcript, summary, and
            notes. This can&apos;t be undone.
          </DialogDescription>
        </DialogHeader>
        <DialogFooter>
          <Button
            variant="outline"
            onClick={() => onOpenChange(false)}
            disabled={isDeleting}
          >
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
