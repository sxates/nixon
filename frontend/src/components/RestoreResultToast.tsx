'use client';

import { useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { toast } from 'sonner';
import { restoreToast, type RestoreResult } from '@/lib/backup';

/** Rust hands the restore result over once and deletes it, so ask once per app session. */
let asked = false;

/** Test seam: forget that the result was already read. */
export function resetRestoreResultAsked() {
  asked = false;
}

/**
 * After a Replace import the app restarts; this shows "Restored N meetings from <date>." (or
 * the failure) the first time the shell mounts (specs/0092). The module-level flag, not an
 * effect-local one, keeps React strict mode's second effect from asking again, and the first
 * run is never cancelled, so the one-shot result can't be lost.
 */
export function RestoreResultToast() {
  useEffect(() => {
    if (asked) return;
    asked = true;
    invoke<RestoreResult | null>('api_backup_last_restore_result')
      .then((r) => {
        if (!r) return;
        const t = restoreToast(r);
        if (t.ok) toast.success(t.text);
        else toast.error(t.text, { duration: 12000 });
      })
      .catch((error) => console.error('Failed to read the restore result:', error));
  }, []);
  return null;
}
