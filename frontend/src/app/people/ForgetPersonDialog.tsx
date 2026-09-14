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

interface ForgetPersonDialogProps {
  /** Controlled open state. */
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** Person to forget. */
  personId: string;
  /** Display name for a friendlier confirmation + success toast. */
  personName?: string;
  /** Called after a successful delete (refetch the list). */
  onForgotten: () => void | Promise<void>;
}

/**
 * Confirm-then-"forget" dialog for a person (specs/0016 1b). Mirrors
 * `DeleteMeetingDialog`'s pattern: the destructive action calls
 * `api_delete_person`, which detaches the person from every speaker (and, per
 * Phase 1c, cascades any stored voiceprints).
 */
export function ForgetPersonDialog({
  open,
  onOpenChange,
  personId,
  personName,
  onForgotten,
}: ForgetPersonDialogProps) {
  const [isDeleting, setIsDeleting] = useState(false);

  const handleConfirm = async () => {
    if (isDeleting) return;
    setIsDeleting(true);
    try {
      await invoke('api_delete_person', { id: personId });
      const label = personName?.trim();
      toast.success(label ? `Forgot ${label}` : 'Person forgotten');
      onOpenChange(false);
      await onForgotten();
    } catch (error) {
      console.error('Failed to forget person:', error);
      toast.error('Could not forget this person. Please try again.', {
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
          <DialogTitle>Forget this person</DialogTitle>
          <DialogDescription>
            Forget{personName?.trim() ? ` ${personName.trim()}` : ' this person'}? This removes them
            from your People directory and unlinks them from any speakers they were assigned to. Any
            stored voice samples are deleted too. This can&apos;t be undone.
          </DialogDescription>
        </DialogHeader>
        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)} disabled={isDeleting}>
            Cancel
          </Button>
          <Button variant="destructive" onClick={handleConfirm} disabled={isDeleting}>
            {isDeleting && <Loader2 className="mr-2 h-4 w-4 animate-spin" />}
            Forget person
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
