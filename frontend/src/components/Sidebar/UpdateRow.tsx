'use client';

import React from 'react';
import { Download, RotateCw } from 'lucide-react';
import { Popover, PopoverContent, PopoverTrigger } from '@/components/ui/popover';
import { cn } from '@/lib/utils';
import { DeckIcon } from '@/components/ui/deck-icon';
import { IconSlot } from './SidebarRow';
import { SIDEBAR_GLYPH, SIDEBAR_ROW } from './row';
import { useOptionalUpdateStatus } from '@/contexts/UpdateStatusContext';
import { useRecordingState } from '@/contexts/RecordingStateContext';
import { RestartToUpdateButton } from '@/components/Updates/RestartToUpdateButton';

/**
 * specs/0058 — the quiet "an update is here" row in the sidebar footer. Hidden unless a
 * download is in flight or a verified payload is staged. A refused install — from here, the
 * tray, or Settings > About — is shown under the row, so the reason is not confined to
 * Settings > About.
 *
 * **specs/0069 W5** — Restart now goes through `RestartToUpdateButton`, which asks the
 * app-wide confirmation instead of installing directly (see `RestartConfirmContext`), and the
 * indicator is a download/restart glyph rather than an amber `LampDot`. The lamp was
 * identical to the Queue's own amber lamp 40px below it on the collapsed rail — two unlabelled
 * dots, no way to tell "an update is ready" from "background work is running" apart, and the
 * whole click handler used to be `install()` with no warning before the restart. See
 * `UpdateGlyph` below.
 */
function UpdateGlyph({ ready }: { ready: boolean }) {
  return (
    <span data-restart-glyph={ready ? 'ready' : 'downloading'} className="text-brand">
      <DeckIcon icon={ready ? RotateCw : Download} size={SIDEBAR_GLYPH} />
    </span>
  );
}

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

  if (collapsed) {
    return (
      <Popover>
        <PopoverTrigger asChild>
          <button
            type="button"
            title={line}
            aria-label={`Update status — ${line}`}
            data-sidebar-row
            className={cn(SIDEBAR_ROW, 'transition-colors hover:bg-key focus:outline-none focus-visible:ring-2 focus-visible:ring-ring')}
          >
            <IconSlot name="update">
              <UpdateGlyph ready={ready} />
            </IconSlot>
          </button>
        </PopoverTrigger>
        <PopoverContent side="right" align="end" aria-label="Update" className="w-72 border-border bg-popover p-3">
          <p className="u-section-label text-[9px]">Update</p>
          <p className="mt-1 text-xs text-foreground">{line}</p>
          {ready && <RestartToUpdateButton label="Restart to update" className="mt-3 w-full px-2 py-1" />}
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
      <div data-sidebar-row className={SIDEBAR_ROW}>
        <IconSlot name="update">
          <UpdateGlyph ready={ready} />
        </IconSlot>
        {/* The right inset belongs to the trailing content, not the row itself — matching
            `QueueRow`'s own trailing status span (`QueueRow.tsx`). The row div stays
            `SIDEBAR_ROW`-only so its className matches the collapsed variant's, slot for
            slot; without this wrapper the `RestartToUpdateButton` sat flush against the
            panel's border while every other row stopped `pr-3.5` short (review finding,
            fix round 2). */}
        <div className="flex min-w-0 flex-1 items-center gap-2 pr-3.5">
          <span className="min-w-0 flex-1 truncate text-[11px] text-muted-foreground">{line}</span>
          {ready && <RestartToUpdateButton />}
        </div>
      </div>
      {updates.error && (
        // Aligned to where labels start (the `w-16` icon column), not the old `pl-5` from
        // before the shared row geometry (specs/0069 W1).
        <p className="truncate pb-1 pl-16 pr-3.5 text-[11px] text-record-ink" title={updates.error}>
          {updates.error}
        </p>
      )}
    </div>
  );
}
