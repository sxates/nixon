'use client';

import React from 'react';
import { Popover, PopoverContent, PopoverTrigger } from '@/components/ui/popover';
import { cn } from '@/lib/utils';
import { LampDot } from '@/components/Transport/LampDot';
import { SIDEBAR_ICON_SLOT, SIDEBAR_ROW } from './row';
import { useOptionalUpdateStatus } from '@/contexts/UpdateStatusContext';
import { useRecordingState } from '@/contexts/RecordingStateContext';

/**
 * specs/0058 — the quiet "an update is here" row in the sidebar footer. Hidden unless a
 * download is in flight or a verified payload is staged. Restart is disabled while a
 * recording runs (the backend refuses too; this just explains why). A refused install —
 * from here or from the tray — is shown under the row, so the reason is not confined to
 * Settings > About.
 *
 * **Collapsed, the lamp opens a flyout; it does not install** (specs/0066, owner report).
 * It used to be a button whose entire click handler was `install()` — so on the icon rail,
 * where the lamp sits directly above the Queue's own amber lamp and carries no label, one
 * click on a dot you were trying to *identify* restarted the app. The recording guard did
 * hold, but "clicking to find out what this is" should never be the same gesture as
 * "relaunch now", and a restart is not undoable. Expanded and collapsed now agree: the row
 * says what is happening, and Restart is a distinct, labelled action.
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
      <Popover>
        <PopoverTrigger asChild>
          <button
            type="button"
            title={line}
            aria-label={`Update status — ${line}`}
            className="mb-1 flex h-10 w-full items-center justify-center transition-colors hover:bg-key focus:outline-none focus-visible:ring-2 focus-visible:ring-ring"
          >
            <LampDot tone="amber" label={line} decorative pulse={!ready} />
          </button>
        </PopoverTrigger>
        <PopoverContent side="right" align="end" aria-label="Update" className="w-72 border-border bg-popover p-3">
          <p className="u-section-label text-[9px]">Update</p>
          <p className="mt-1 text-xs text-foreground">{line}</p>
          {ready && (
            <button
              type="button"
              onClick={() => updates.install()}
              disabled={disabled}
              className={cn(
                'u-section-label mt-3 w-full rounded-[3px] border border-border bg-key px-2 py-1 text-[9px] text-foreground transition-colors',
                'hover:bg-key/80 focus:outline-none focus-visible:ring-2 focus-visible:ring-ring',
                'disabled:cursor-default disabled:opacity-50 disabled:hover:bg-key',
              )}
            >
              Restart to update
            </button>
          )}
          {/* Said out loud, not hidden in a `title`: on the icon rail there is no row text
              to explain why the button is dead. */}
          {ready && isRecording && (
            <p className="mt-2 text-[11px] text-muted-foreground">
              Finish the recording first — Nixon won&apos;t restart mid-take.
            </p>
          )}
          {updates.error && <p className="mt-2 text-[11px] text-record-ink">{updates.error}</p>}
        </PopoverContent>
      </Popover>
    );
  }

  return (
    <div>
      <div className={cn(SIDEBAR_ROW, 'h-8')}>
        <span className={SIDEBAR_ICON_SLOT}>
          <LampDot tone="amber" label={line} decorative pulse={!ready} />
        </span>
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
