/**
 * RunStatus (docs/design-system.md §2) — a run in progress (an Ask, a rollup, a brief):
 * the brand spinner, a live label and an optional Cancel, inside a `Panel`. The label is the
 * `role="status"` region, so stage changes are announced.
 */

import * as React from 'react';
import { Button } from '@/components/ui/button';
import { Panel } from '@/components/ui/panel';
import { Spinner } from '@/components/ui/spinner';
import { cn } from '@/lib/utils';

export interface RunStatusProps {
  /** What is happening right now ("Searching 12 meetings…"). */
  label: React.ReactNode;
  /** Shows a Cancel button. */
  onCancel?: () => void;
  /** The cancel is in flight: the button reads "Cancelling…" and is disabled. */
  cancelling?: boolean;
  cancelLabel?: string;
  className?: string;
}

/** A run in progress — spinner, live label and an optional Cancel — inside a `Panel`. */
export function RunStatus({
  label,
  onCancel,
  cancelling = false,
  cancelLabel = 'Cancel',
  className,
}: RunStatusProps) {
  return (
    <Panel className={cn('flex items-center justify-between gap-3', className)}>
      <p role="status" className="flex min-w-0 items-center gap-2 text-sm text-foreground">
        <Spinner className="text-brand" />
        <span className="truncate">{label}</span>
      </p>
      {onCancel ? (
        <Button variant="outline" size="sm" onClick={onCancel} disabled={cancelling} className="flex-none">
          {cancelling ? 'Cancelling…' : cancelLabel}
        </Button>
      ) : null}
    </Panel>
  );
}
