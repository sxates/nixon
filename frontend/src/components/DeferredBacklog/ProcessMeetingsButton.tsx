'use client';

import { Button } from '@/components/ui/button';
import { Spinner } from '@/components/ui/spinner';
import { StatusDot } from '@/components/ui/status-dot';
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
    <Button variant="outline" onClick={() => setOpen(true)}>
      <Spinner size="sm" />
      Processing… ({view.activeOrdinal} of {view.total})
    </Button>
  ) : (
    <Button variant="outline" onClick={startNow}>
      <StatusDot size={6} tone="brand" />
      Process {view.pendingCount} meeting{view.pendingCount === 1 ? '' : 's'}
    </Button>
  );
}
