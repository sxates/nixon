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
import type { SummaryPrompt } from '@/lib/summary-prompts';

interface DeletePromptDialogProps {
  /** Controlled open state. */
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** Prompt to delete — never the default (the backend rejects that). */
  prompt: SummaryPrompt | null;
  /** Called after a successful delete so the caller can refresh its list. */
  onDeleted: () => void | Promise<void>;
}

/** Confirm-then-delete for a saved summary prompt (specs/0079 W4). */
export function DeletePromptDialog({ open, onOpenChange, prompt, onDeleted }: DeletePromptDialogProps) {
  const [isDeleting, setIsDeleting] = useState(false);

  const handleConfirm = async () => {
    if (isDeleting || !prompt) return;
    setIsDeleting(true);
    try {
      await invoke('api_delete_summary_prompt', { id: prompt.id });
      toast.success(`Deleted "${prompt.name}"`);
      onOpenChange(false);
      await onDeleted();
    } catch (error) {
      console.error('Failed to delete summary prompt:', error);
      toast.error('Could not delete prompt', {
        description:
          typeof error === 'string'
            ? error
            : error instanceof Error
              ? error.message
              : 'Please try again.',
      });
    } finally {
      setIsDeleting(false);
    }
  };

  return (
    <Dialog open={open} onOpenChange={(next) => (isDeleting ? undefined : onOpenChange(next))}>
      <DialogContent className="max-w-md">
        <DialogHeader>
          <DialogTitle>Delete prompt?</DialogTitle>
          <DialogDescription>
            {`Delete "${prompt?.name}"? Meetings that used it keep their summaries; future summaries use the default prompt. This can't be undone.`}
          </DialogDescription>
        </DialogHeader>
        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)} disabled={isDeleting}>
            Cancel
          </Button>
          <Button variant="destructive" onClick={handleConfirm} disabled={isDeleting || !prompt}>
            {isDeleting && <Loader2 className="mr-2 h-4 w-4 animate-spin" />}
            Delete
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
