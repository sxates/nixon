'use client';

/**
 * "Import a backup" (specs/0092): the preview of the picked file and the two ways in.
 * Add what's missing is the default when this Mac already has meetings; with none, there is
 * nothing to add to, so Replace everything… is the only (primary) choice. Replace asks once
 * more, because it overwrites this Mac's data (a safety copy is saved first).
 */

import { useState } from 'react';
import { Button } from '@/components/ui/button';
import { ConfirmDialog } from '@/components/ui/confirm-dialog';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog';
import { formatBackupDate, previewLine, type BackupPreview } from '@/lib/backup';

interface BackupImportDialogProps {
  /** The previewed file; null closes the dialog. */
  preview: BackupPreview | null;
  onClose: () => void;
  onAdd: () => void;
  onReplace: () => void;
}

export function BackupImportDialog({ preview, onClose, onAdd, onReplace }: BackupImportDialogProps) {
  const [confirming, setConfirming] = useState(false);
  if (!preview) return null;
  const hasLocal = preview.localMeetings > 0;

  return (
    <>
      <Dialog open={!confirming} onOpenChange={(open) => !open && onClose()}>
        <DialogContent className="max-w-md">
          <DialogHeader>
            <DialogTitle>Import a backup</DialogTitle>
            <DialogDescription>{previewLine(preview)}</DialogDescription>
          </DialogHeader>
          <p className="text-sm text-foreground">
            {hasLocal
              ? "Add what's missing keeps everything on this Mac and brings in what the backup has that you don't. Replace everything makes this Mac match the backup."
              : 'This Mac has no meetings yet, so the backup replaces what is here.'}
          </p>
          <DialogFooter>
            <Button variant="outline" onClick={onClose}>
              Cancel
            </Button>
            <Button variant={hasLocal ? 'outline' : 'brand'} onClick={() => setConfirming(true)}>
              Replace everything…
            </Button>
            {hasLocal ? (
              <Button variant="brand" onClick={onAdd}>
                {"Add what's missing"}
              </Button>
            ) : null}
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <ConfirmDialog
        open={confirming}
        onOpenChange={(open) => {
          if (!open) setConfirming(false);
        }}
        title="Replace everything?"
        body={`This replaces all meetings, notes, To-Dos, Topics and People on this Mac with the backup from ${formatBackupDate(preview.createdAt)}. A copy of your current data is saved first. Nixon restarts.`}
        confirmLabel="Replace everything"
        onConfirm={() => {
          setConfirming(false);
          onReplace();
        }}
      />
    </>
  );
}
