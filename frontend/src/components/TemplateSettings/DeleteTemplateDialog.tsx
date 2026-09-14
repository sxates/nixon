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
import { TemplateListItem } from './types';

interface DeleteTemplateDialogProps {
  /** Controlled open state. */
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** Template to delete — only 'custom' and 'override' entries ever get here. */
  template: TemplateListItem | null;
  /** Called after a successful delete so the caller can refresh its list. */
  onDeleted: () => void | Promise<void>;
}

/**
 * Confirm-then-delete for custom/edited templates (specs/0020 task 9). Deleting an
 * "Edited" entry (a built-in override) reverts to the original built-in; deleting a
 * custom template removes it entirely (meetings that used it fall back to the
 * default template at generation time). Built-ins are never deletable.
 */
export function DeleteTemplateDialog({
  open,
  onOpenChange,
  template,
  onDeleted,
}: DeleteTemplateDialogProps) {
  const [isDeleting, setIsDeleting] = useState(false);
  const isOverride = template?.source === 'override';

  const handleConfirm = async () => {
    if (isDeleting || !template) return;
    setIsDeleting(true);
    try {
      await invoke('api_delete_template', { id: template.id });
      toast.success(
        isOverride ? `Reverted "${template.name}" to the built-in version` : `Deleted "${template.name}"`,
      );
      onOpenChange(false);
      await onDeleted();
    } catch (error) {
      console.error('Failed to delete template:', error);
      const message =
        typeof error === 'string'
          ? error
          : error instanceof Error
            ? error.message
            : 'Please try again.';
      toast.error('Could not delete template', { description: message });
    } finally {
      setIsDeleting(false);
    }
  };

  return (
    <Dialog open={open} onOpenChange={(next) => (isDeleting ? undefined : onOpenChange(next))}>
      <DialogContent className="max-w-md">
        <DialogHeader>
          <DialogTitle>{isOverride ? 'Revert template?' : 'Delete template?'}</DialogTitle>
          <DialogDescription>
            {isOverride
              ? `"${template?.name}" is an edited built-in template. Deleting your edits reverts it to the original built-in version.`
              : `Delete "${template?.name}"? Meetings that used it keep their summaries; future generations fall back to the default template. This can't be undone.`}
          </DialogDescription>
        </DialogHeader>
        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)} disabled={isDeleting}>
            Cancel
          </Button>
          <Button variant="destructive" onClick={handleConfirm} disabled={isDeleting || !template}>
            {isDeleting && <Loader2 className="mr-2 h-4 w-4 animate-spin" />}
            {isOverride ? 'Revert to built-in' : 'Delete'}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
