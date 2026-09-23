'use client';

import { useEffect, useState } from 'react';
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
import { tildePath } from '@/lib/format-path';
import {
  errorText,
  formatBytes,
  isICloudDrivePath,
  plural,
  type MovePlan,
} from '@/lib/recordings-move';

/** Free space the backend keeps in reserve on top of a cross-drive copy. */
const SPACE_MARGIN_BYTES = 1024 ** 3;

interface MoveRecordingsDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  plan: MovePlan | null;
  /** `change`: the user just picked a new folder. `gather`: the first launch found
   *  recordings outside the current folder. */
  mode: 'change' | 'gather';
  /** Starts the move. A rejection keeps the dialog open and shows its text. */
  onConfirm: () => Promise<void>;
}

/**
 * "Move your recordings?" (specs/0073). Nixon keeps every recording in one folder, so
 * the only choices are Move recordings and Cancel — there is deliberately no "only new
 * recordings" option. Cancel changes nothing.
 *
 * Paths render through `tildePath` so the dialog never puts the account name on screen.
 */
export function MoveRecordingsDialog({
  open,
  onOpenChange,
  plan,
  mode,
  onConfirm,
}: MoveRecordingsDialogProps) {
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (open) setError(null);
  }, [open, plan]);

  if (!plan) return null;

  const handleConfirm = async () => {
    if (submitting) return;
    setSubmitting(true);
    setError(null);
    try {
      await onConfirm();
      onOpenChange(false);
    } catch (e) {
      setError(errorText(e));
    } finally {
      setSubmitting(false);
    }
  };

  const where = tildePath(plan.target);
  const what = `${plural(plan.meetings, 'meeting')}, ${formatBytes(plan.bytes)}`;
  const needed = plan.crossVolumeBytes + SPACE_MARGIN_BYTES;

  return (
    <Dialog open={open} onOpenChange={(next) => (submitting ? undefined : onOpenChange(next))}>
      <DialogContent className="max-w-md">
        <DialogHeader>
          <DialogTitle>Move your recordings?</DialogTitle>
          <DialogDescription>
            {mode === 'gather'
              ? 'Some of your recordings are in other folders. Nixon keeps them all in one place.'
              : 'Nixon keeps all your recordings in one folder, so the existing ones move too.'}
          </DialogDescription>
        </DialogHeader>

        <div className="space-y-2 text-sm text-foreground">
          <p>
            {what}, will move to{' '}
            <span className="break-all font-medium" title={where}>
              {where}
            </span>
            .
          </p>
          {plan.crossVolumeCount > 0 && (
            <p className="text-muted-foreground">
              This is a copy across drives and may take a while.
            </p>
          )}
          {plan.elsewhere > 0 && (
            <p className="text-muted-foreground">
              Includes {plan.elsewhere} from other folders.
            </p>
          )}
          {plan.missing > 0 && (
            <p className="text-muted-foreground">
              {plural(plan.missing, 'meeting')} {plan.missing === 1 ? 'has' : 'have'} no
              recording on disk and will stay as {plan.missing === 1 ? 'it is' : 'they are'}.
            </p>
          )}
          {isICloudDrivePath(plan.target) && (
            <p className="text-muted-foreground">
              iCloud Drive can remove the copies on this Mac to save space. Nixon needs the
              files on this Mac to play and transcribe them.
            </p>
          )}
          {!plan.enoughSpace && (
            <p role="alert" className="text-destructive">
              There isn&apos;t enough free space there. The move needs about{' '}
              {formatBytes(needed)} and {formatBytes(plan.freeBytes)} is free.
            </p>
          )}
          {error && (
            <p role="alert" className="text-destructive">
              {error}
            </p>
          )}
        </div>

        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)} disabled={submitting}>
            Cancel
          </Button>
          <Button onClick={handleConfirm} disabled={submitting || !plan.enoughSpace}>
            {submitting && <Loader2 className="mr-2 h-4 w-4 animate-spin" />}
            Move recordings
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
