'use client';

import { useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { toast } from 'sonner';
import { ConfirmDialog } from '@/components/ui/confirm-dialog';

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
    <ConfirmDialog
      open={open}
      onOpenChange={onOpenChange}
      title="Clear all voiceprints"
      body={
        <>
          Delete every stored voice sample, for everyone? This resets cross-meeting voice
          recognition — the people in your directory and their names stay, but Nixon will have to
          re-learn voices from scratch. This can&apos;t be undone.
        </>
      }
      confirmLabel="Clear all voiceprints"
      busy={isClearing}
      onConfirm={handleConfirm}
    />
  );
}
