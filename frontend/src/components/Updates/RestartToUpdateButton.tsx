'use client';

import React from 'react';
import { useRestartConfirm } from '@/contexts/RestartConfirmContext';
import { useRecordingState } from '@/contexts/RecordingStateContext';
import { Button } from '@/components/ui/button';
import { cn } from '@/lib/utils';

/**
 * Asks for the restart confirmation (specs/0069 W5). It has no `install` of its own — that
 * is the point: every entry point routes through the one dialog, so there is no click
 * anywhere in Nixon that relaunches the app on its own.
 */
export function RestartToUpdateButton({
  label = 'Restart',
  className,
}: {
  label?: string;
  className?: string;
}) {
  const { request, canRestart } = useRestartConfirm();
  const { isRecording } = useRecordingState();
  return (
    // An engraved key cap: the outline button at its smallest size, labelled in the 9px
    // engraved caps (`.u-section-label-sm`, sized by `text-micro`).
    <Button
      variant="outline"
      size="xs"
      onClick={request}
      disabled={!canRestart}
      title={isRecording ? 'Finish the recording first' : undefined}
      className={cn(
        'u-section-label-sm px-2 text-micro text-foreground',
        // Keep the hover while disabled, so the "Finish the recording first" title still shows.
        'disabled:pointer-events-auto disabled:cursor-default',
        className,
      )}
    >
      {label}
    </Button>
  );
}
