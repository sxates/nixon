'use client';

import React from 'react';
import { cn } from '@/lib/utils';
import { LampDot } from '@/components/Transport/LampDot';
import { useOptionalUpdateStatus } from '@/contexts/UpdateStatusContext';
import { useRecordingState } from '@/contexts/RecordingStateContext';

/**
 * specs/0058 — the quiet "an update is here" row in the sidebar footer. Hidden unless a
 * download is in flight or a verified payload is staged. Restart is disabled while a
 * recording runs (the backend refuses too; this just explains why). A refused install —
 * from here or from the tray — is shown under the row, so the reason is not confined to
 * Settings > About.
 */
export function UpdateRow({ collapsed = false }: { collapsed?: boolean }) {
  const updates = useOptionalUpdateStatus();
  const { isRecording } = useRecordingState();
  const status = updates?.status;
  if (!updates || !status || (status.state !== 'downloading' && status.state !== 'ready')) return null;

  const ready = status.state === 'ready';
  const percent =
    status.state === 'downloading' && status.total ? Math.min(100, Math.round((status.received / status.total) * 100)) : null;
  const line = ready
    ? `Nixon ${status.version} ready`
    : percent === null
      ? `Downloading ${status.version}`
      : `Downloading ${status.version} · ${percent}%`;
  const disabled = isRecording || updates.busy;
  const title = isRecording ? 'Finish the recording first' : undefined;

  if (collapsed) {
    return (
      <button
        type="button"
        onClick={() => updates.install()}
        disabled={!ready || disabled}
        title={title ?? line}
        aria-label={ready ? `Restart to update to ${status.version}` : line}
        className="mb-1 flex h-10 w-full items-center justify-center transition-colors hover:bg-key focus:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:cursor-default disabled:hover:bg-transparent"
      >
        <LampDot tone="amber" label={line} decorative pulse={!ready} />
      </button>
    );
  }

  return (
    <div>
      <div className="flex h-8 items-center gap-2.5 pl-3 pr-2">
        <LampDot tone="amber" label={line} decorative pulse={!ready} />
        <span className="min-w-0 flex-1 truncate text-[11px] text-muted-foreground">{line}</span>
        {ready && (
          <button
            type="button"
            onClick={() => updates.install()}
            disabled={disabled}
            title={title}
            className={cn(
              'u-section-label rounded-[3px] border border-border bg-key px-2 py-0.5 text-[9px] text-foreground transition-colors',
              'hover:bg-key/80 focus:outline-none focus-visible:ring-2 focus-visible:ring-ring',
              'disabled:cursor-default disabled:opacity-50 disabled:hover:bg-key',
            )}
          >
            Restart
          </button>
        )}
      </div>
      {updates.error && (
        <p className="truncate pb-1 pl-3 pr-2 text-[11px] text-record-ink" title={updates.error}>
          {updates.error}
        </p>
      )}
    </div>
  );
}
