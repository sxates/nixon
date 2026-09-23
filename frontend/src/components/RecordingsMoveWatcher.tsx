'use client';

import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { toast } from 'sonner';
import { safeListen } from '@/lib/safe-listen';
import { MoveRecordingsDialog } from '@/components/MoveRecordingsDialog';
import {
  finishToast,
  isGatherState,
  isMoveFinished,
  isMovePlan,
  type MovePlan,
} from '@/lib/recordings-move';

/** Asked once per app session, even across a webview remount. */
export const GATHER_ASKED_KEY = 'nixon.recordingsGatherAsked';

function alreadyAsked(): boolean {
  try {
    return sessionStorage.getItem(GATHER_ASKED_KEY) === '1';
  } catch {
    return false;
  }
}

function markAsked() {
  try {
    sessionStorage.setItem(GATHER_ASKED_KEY, '1');
  } catch {
    // Storage unavailable: the worst case is asking again after a reload.
  }
}

/**
 * App-wide half of the recordings mover's UI (specs/0073 W3), mounted in AppShell so it
 * works on every route:
 *
 * - the finish toast, for moves started in Settings AND the silent startup gather;
 * - the first-launch question: the first time this version finds recordings outside the
 *   recordings folder, it asks before moving anything (Move recordings / Cancel). Cancel
 *   moves nothing; Settings keeps offering "Move them here".
 *
 * The question arrives two ways because the startup event can fire before this listens:
 * a pull of `api_recordings_gather_state` on mount, and the `recordings-gather-needed` event.
 */
export function RecordingsMoveWatcher() {
  const [askPlan, setAskPlan] = useState<MovePlan | null>(null);

  useEffect(() => {
    let cancelled = false;
    const ask = (plan: unknown) => {
      if (cancelled || !isMovePlan(plan) || plan.meetings <= 0 || alreadyAsked()) return;
      markAsked();
      setAskPlan(plan);
    };

    invoke<unknown>('api_recordings_gather_state')
      .then((g) => {
        if (isGatherState(g) && g.needsConfirmation) ask(g.plan);
      })
      .catch(() => {});

    const offs = [
      safeListen<{ plan?: unknown }>('recordings-gather-needed', (e) => ask(e.payload?.plan)),
      safeListen<unknown>('recordings-move-finished', (e) => {
        if (!isMoveFinished(e.payload)) return;
        const t = finishToast(e.payload);
        if (!t) return;
        const show = t.show;
        toast[t.kind](t.title, {
          description: t.description,
          duration: t.kind === 'warning' ? 12000 : undefined,
          action: show
            ? {
                label: 'Show',
                onClick: () => {
                  invoke('open_meeting_folder', { meetingId: show.meetingId }).catch((error) =>
                    console.error('Failed to open the recording folder:', error),
                  );
                },
              }
            : undefined,
        });
      }),
    ];
    return () => {
      cancelled = true;
      offs.forEach((off) => off());
    };
  }, []);

  return (
    <MoveRecordingsDialog
      open={!!askPlan}
      onOpenChange={(open) => !open && setAskPlan(null)}
      plan={askPlan}
      mode="gather"
      onConfirm={async () => {
        await invoke('api_gather_recordings');
      }}
    />
  );
}
