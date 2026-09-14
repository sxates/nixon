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

interface ClearVoiceprintsDialogProps {
  /** Controlled open state. */
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** Called after a successful clear (so the caller can refresh any counts). */
  onCleared?: (count: number) => void | Promise<void>;
}

/**
 * Confirm-then-"clear all voiceprints" dialog (ADR-0007 §6). Mirrors
 * `DeleteMeetingDialog`'s pattern. Calls `api_clear_all_voiceprints`, which wipes
 * every stored voice sample (gallery reset) while leaving People/identity intact,
 * and returns the number of samples deleted for the success toast.
 */
export function ClearVoiceprintsDialog({
  open,
  onOpenChange,
  onCleared,
}: ClearVoiceprintsDialogProps) {
  const [isClearing, setIsClearing] = useState(false);

  const handleConfirm = async () => {
    if (isClearing) return;
    setIsClearing(true);
    try {
      const count = await invoke<number>('api_clear_all_voiceprints');
      const n = typeof count === 'number' ? count : 0;
      toast.success(
        n === 1 ? 'Cleared 1 voice sample.' : `Cleared ${n} voice samples.`,
      );
      onOpenChange(false);
      await onCleared?.(n);
    } catch (error) {
      console.error('Failed to clear voiceprints:', error);
      toast.error('Could not clear voice samples. Please try again.', {
        description: error instanceof Error ? error.message : String(error),
      });
    } finally {
      setIsClearing(false);
    }
  };

  return (
    <Dialog open={open} onOpenChange={(next) => (isClearing ? undefined : onOpenChange(next))}>
      <DialogContent className="max-w-md">
        <DialogHeader>
          <DialogTitle>Clear all voiceprints</DialogTitle>
          <DialogDescription>
            Delete every stored voice sample, for everyone? This resets cross-meeting voice
            recognition — the people in your directory and their names stay, but Nixon will have to
            re-learn voices from scratch. This can&apos;t be undone.
          </DialogDescription>
        </DialogHeader>
        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)} disabled={isClearing}>
            Cancel
          </Button>
          <Button variant="destructive" onClick={handleConfirm} disabled={isClearing}>
            {isClearing && <Loader2 className="mr-2 h-4 w-4 animate-spin" />}
            Clear all voiceprints
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
