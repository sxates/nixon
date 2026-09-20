'use client';

import React, { createContext, useCallback, useContext, useEffect, useMemo, useState } from 'react';
import { getVersion } from '@tauri-apps/api/app';
import { toast } from 'sonner';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog';
import { Button } from '@/components/ui/button';
import { useOptionalUpdateStatus } from '@/contexts/UpdateStatusContext';
import { useRecordingState } from '@/contexts/RecordingStateContext';
import { safeListen } from '@/lib/safe-listen';

/**
 * specs/0069 W5 — the one place Nixon agrees to restart.
 *
 * Restarting is not undoable, and until this existed it happened on the click a user
 * pressed to find out what the indicator meant, with nothing anywhere saying the app was
 * about to close. There were four ways to trigger it — the sidebar row, the collapsed
 * rail's flyout, Settings → About, and the menu-bar tray, which calls the Rust installer
 * directly and has no web layer to put a dialog in. So the dialog is mounted app-wide and
 * every path asks it: the three in the UI through `useRestartConfirm()`, the tray by
 * emitting `update-confirm-restart` instead of installing.
 *
 * The recording guard is belt and braces: the button is disabled, this refuses to open, and
 * `install_staged` re-checks on the Rust side immediately before the point of no return.
 */
interface RestartConfirmValue {
  request: () => void;
  canRestart: boolean;
}

const Ctx = createContext<RestartConfirmValue | null>(null);

export function RestartConfirmProvider({ children }: { children: React.ReactNode }) {
  const updates = useOptionalUpdateStatus();
  const { isRecording } = useRecordingState();
  const [open, setOpen] = useState(false);
  const [current, setCurrent] = useState<string | null>(null);

  const status = updates?.status;
  const staged = status?.state === 'ready' ? status : null;
  const canRestart = !!staged && !isRecording && !updates?.busy;

  useEffect(() => {
    // Outside Tauri (tests, plain browser) there is no version to read; the dialog omits
    // the "from" half rather than refusing to open.
    getVersion().then(setCurrent).catch(() => setCurrent(null));
  }, []);

  const request = useCallback(() => {
    if (!canRestart) {
      // Review finding, fix round 2: a refused request used to fail in total silence — the
      // in-UI buttons are `disabled` so this only fires from the tray (which has no web
      // layer of its own to put a dialog in), in the narrow window where the menu item was
      // built while `Stopped` but a recording started, or an install is already running, by
      // the time the click lands. Before `RestartConfirmContext` existed the same click
      // reached `install_and_restart`, whose refusal surfaced under the sidebar row and in
      // About via `update-install-refused` — this keeps that "something happened" promise
      // without a round trip through Rust, reusing the recording copy verbatim
      // (`UpdateRow.tsx`'s popover).
      if (isRecording) {
        toast.error("Finish the recording first — Nixon won't restart mid-take.");
      } else if (updates?.busy) {
        toast.error('Nixon is already installing an update.');
      } else {
        toast.error('No update is ready to install.');
      }
      return;
    }
    setOpen(true);
  }, [canRestart, isRecording, updates?.busy]);

  // The tray item (tray.rs, "install_update") emits this instead of installing.
  useEffect(() => {
    const unlisten = safeListen('update-confirm-restart', () => request());
    return () => unlisten();
  }, [request]);

  // A staged payload that disappears (a failed verify clears it) must not leave a dialog
  // open offering to install nothing.
  useEffect(() => {
    if (!canRestart) setOpen(false);
  }, [canRestart]);

  const value = useMemo(() => ({ request, canRestart }), [request, canRestart]);

  return (
    <Ctx.Provider value={value}>
      {children}
      <Dialog open={open} onOpenChange={setOpen}>
        <DialogContent className="max-w-md">
          <DialogHeader>
            <DialogTitle>Restart to update?</DialogTitle>
            <DialogDescription>
              {current && staged
                ? `Nixon ${current} → ${staged.version}. `
                : staged
                  ? `Nixon ${staged.version}. `
                  : ''}
              Nixon will close and reopen; your meetings, notes and settings stay as they are.
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="outline" onClick={() => setOpen(false)}>
              Cancel
            </Button>
            <Button
              variant="brand"
              onClick={() => {
                setOpen(false);
                void updates?.install();
              }}
            >
              Restart now
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </Ctx.Provider>
  );
}

/** Tolerates a tree without the provider (unit tests of a single row). */
export function useRestartConfirm(): RestartConfirmValue {
  return useContext(Ctx) ?? { request: () => {}, canRestart: false };
}
