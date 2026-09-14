'use client';

import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog';
import { VoiceprintSamplesList } from '@/components/People/VoiceprintSamplesList';

interface VoiceprintSamplesDialogProps {
  /** Controlled open state. */
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** Person whose voice samples to manage. */
  personId: string;
  /** Display name for the header + toasts. */
  personName?: string;
}

/**
 * Per-sample voice gallery management for one person, in a dialog (specs/0039 WS3, task 9).
 *
 * The gallery body (list + quarantine/restore/delete) lives in the reusable
 * `VoiceprintSamplesList` (shared with the person-detail page's "Voice Samples" tab,
 * specs/0038 dogfood feedback #3); this shell just frames it in a modal. Radix unmounts the
 * content on close, so the list remounts — and refetches a fresh gallery — on each open.
 * `bounded` caps the list at 52vh with native scrolling so a long gallery scrolls inside the
 * modal instead of growing it off-screen (specs/0056 W5).
 */
export function VoiceprintSamplesDialog({
  open,
  onOpenChange,
  personId,
  personName,
}: VoiceprintSamplesDialogProps) {
  const label = personName?.trim() || 'this person';

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-lg">
        <DialogHeader>
          <DialogTitle>Voice samples</DialogTitle>
          <DialogDescription>
            Every stored voice sample Nixon learned for {label}. Quarantine a sample to stop it
            affecting voice matching (recoverable), or delete it permanently.
          </DialogDescription>
        </DialogHeader>

        <VoiceprintSamplesList personId={personId} personName={personName} bounded />
      </DialogContent>
    </Dialog>
  );
}
