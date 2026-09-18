'use client';

import { Loader2 } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { useBacklog } from '@/contexts/DeferredBacklogProvider';
import { useQueueOpen } from '@/contexts/QueueOpenContext';

/**
 * Prominent "Process meetings" control for the Today header (spec 0045 WS3), mirroring the
 * Record button's slot. Shown only when a backlog exists. Idle → "Process N meetings" starts
 * the drain; running → "Processing… (k of N)" opens the ONE queue popover the Transport rail
 * also opens (specs/0063 W3 Task 6 — Today no longer keeps a second, duplicate queue popover
 * of its own). State comes entirely from the shared controller, so it never shows a fake
 * spinner and survives navigation.
 */
export function ProcessMeetingsButton() {
  const { view, startNow } = useBacklog();
  const { setOpen } = useQueueOpen();
  if (view.pendingCount === 0 && !view.processing) return null;

  return view.processing ? (
    <Button variant="outline" onClick={() => setOpen(true)} className="gap-2">
      <Loader2 size={14} className="animate-spin" aria-hidden />
      Processing… ({view.activeOrdinal} of {view.total})
    </Button>
  ) : (
    <Button variant="outline" onClick={startNow} className="gap-2">
      <span className="h-1.5 w-1.5 rounded-full bg-brand" aria-hidden />
      Process {view.pendingCount} meeting{view.pendingCount === 1 ? '' : 's'}
    </Button>
  );
}
