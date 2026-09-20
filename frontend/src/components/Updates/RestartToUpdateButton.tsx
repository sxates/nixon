'use client';

import React from 'react';
import { useRestartConfirm } from '@/contexts/RestartConfirmContext';
import { useRecordingState } from '@/contexts/RecordingStateContext';
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
    <button
      type="button"
      onClick={request}
      disabled={!canRestart}
      title={isRecording ? 'Finish the recording first' : undefined}
      className={cn(
        'u-section-label rounded-[3px] border border-border bg-key px-2 py-0.5 text-[9px] text-foreground transition-colors',
        'hover:bg-key/80 focus:outline-none focus-visible:ring-2 focus-visible:ring-ring',
        'disabled:cursor-default disabled:opacity-50 disabled:hover:bg-key',
        className,
      )}
    >
      {label}
    </button>
  );
}
