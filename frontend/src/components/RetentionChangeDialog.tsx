'use client';

import { useState } from 'react';
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
import { formatBytes, plural } from '@/lib/recordings-move';
import {
  keptForNowSentence,
  retentionDeletesNowSentence,
  type AudioRetentionChoice,
  type RetentionPreview,
} from '@/lib/audio-retention';

interface RetentionChangeDialogProps {
  /** The choice being confirmed and its dry run; the dialog is open while both are set. */
  pending: { choice: AudioRetentionChoice; preview: RetentionPreview } | null;
  /** Saves, deletes, reports. A rejection keeps the dialog open. */
  onConfirm: () => Promise<void>;
  onCancel: () => void;
}

/**
 * "Delete audio from N meetings now?" (specs/0072 W3). Shown only when shortening how long
 * audio is kept would delete something right away. Delete audio saves the setting and
 * deletes at once; Cancel saves nothing. Deletion is final (no Trash), so the dialog says so.
 */
export function RetentionChangeDialog({ pending, onConfirm, onCancel }: RetentionChangeDialogProps) {
  const [submitting, setSubmitting] = useState(false);
  const kept = pending ? keptForNowSentence(pending.preview) : null;

  const handleConfirm = async () => {
    if (submitting) return;
    setSubmitting(true);
    try {
      await onConfirm();
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <Dialog
      open={pending !== null}
      onOpenChange={(open) => {
        if (!open && !submitting) onCancel();
      }}
    >
      <DialogContent className="max-w-md">
        <DialogHeader>
          <DialogTitle>
            {pending
              ? `Delete audio from ${plural(pending.preview.meetings, 'meeting')} (${formatBytes(pending.preview.bytes)}) now?`
              : 'Delete audio now?'}
          </DialogTitle>
          <DialogDescription>
            {pending && retentionDeletesNowSentence(pending.choice)} Transcripts, notes, summaries
            and speaker names are kept. This can&apos;t be undone.
          </DialogDescription>
        </DialogHeader>
        {kept && <p className="text-xs text-muted-foreground">{kept}</p>}
        <DialogFooter>
          <Button variant="outline" onClick={onCancel} disabled={submitting}>
            Cancel
          </Button>
          <Button variant="destructive" onClick={() => void handleConfirm()} disabled={submitting}>
            {submitting && <Loader2 className="mr-2 h-4 w-4 animate-spin" />}
            Delete audio
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
