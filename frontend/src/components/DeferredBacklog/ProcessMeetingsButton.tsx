'use client';

import { useState } from 'react';
import { Loader2 } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { useBacklog } from '@/contexts/DeferredBacklogProvider';
import { BacklogDetailPopover } from './BacklogDetailPopover';

/**
 * Prominent "Process meetings" control for the Today header (spec 0045 WS3), mirroring the
 * Record button's slot. Shown only when a backlog exists. Idle → "Process N meetings" starts
 * the drain; running → "Processing… (k of N)" opens the status popover. State comes entirely
 * from the shared controller, so it never shows a fake spinner and survives navigation.
 */
export function ProcessMeetingsButton() {
  const { view, startNow } = useBacklog();
  const [open, setOpen] = useState(false);
  if (view.pendingCount === 0 && !view.processing) return null;

  return (
    <div className="relative">
      {view.processing ? (
        <Button variant="outline" onClick={() => setOpen((o) => !o)} className="gap-2">
          <Loader2 size={14} className="animate-spin" aria-hidden />
          Processing… ({view.activeOrdinal} of {view.total})
        </Button>
      ) : (
        <Button variant="outline" onClick={startNow} className="gap-2">
          <span className="h-1.5 w-1.5 rounded-full bg-brand" aria-hidden />
          Process {view.pendingCount} meeting{view.pendingCount === 1 ? '' : 's'}
        </Button>
      )}
      {open && (
        <div className="absolute right-0 top-11 z-50">
          <BacklogDetailPopover onClose={() => setOpen(false)} />
        </div>
      )}
    </div>
  );
}
