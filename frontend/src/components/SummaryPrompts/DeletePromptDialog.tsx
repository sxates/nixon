'use client';

import { useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { toast } from 'sonner';
import { ConfirmDialog } from '@/components/ui/confirm-dialog';
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
    <ConfirmDialog
      open={open}
      onOpenChange={onOpenChange}
      title="Delete prompt?"
      body={`Delete "${prompt?.name}"? Meetings that used it keep their summaries; future summaries use the default prompt. This can't be undone.`}
      confirmLabel="Delete"
      busy={isDeleting}
      confirmDisabled={!prompt}
      onConfirm={handleConfirm}
    />
  );
}
