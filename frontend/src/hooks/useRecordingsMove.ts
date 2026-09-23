'use client';

import { useCallback, useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { toast } from 'sonner';
import { safeListen } from '@/lib/safe-listen';
import {
  errorText,
  isGatherState,
  isMoveFinished,
  isMoveStatus,
  type GatherState,
  type MoveStatus,
} from '@/lib/recordings-move';

/**
 * The recordings mover's state for the Save location row (specs/0073 W3).
 *
 * The move runs in Rust and outlives this component, so on mount it PULLS the current
 * state (`api_recordings_move_status`, `api_recordings_gather_state`) and only then relies
 * on events. That is what lets leaving and returning to Settings re-attach to a running
 * move, and what catches the startup gather's events, which fire before any webview
 * listener exists.
 */
export function useRecordingsMove() {
  const [status, setStatus] = useState<MoveStatus | null>(null);
  const [gather, setGather] = useState<GatherState | null>(null);
  const [stopping, setStopping] = useState(false);
  const [lastFailure, setLastFailure] = useState<string | null>(null);

  const refreshGather = useCallback(async () => {
    try {
      const g = await invoke<unknown>('api_recordings_gather_state');
      setGather(isGatherState(g) ? g : null);
    } catch (error) {
      console.error('Failed to read the recordings folder state:', error);
    }
  }, []);

  useEffect(() => {
    let cancelled = false;
    invoke<unknown>('api_recordings_move_status')
      .then((s) => {
        if (!cancelled) setStatus(isMoveStatus(s) ? s : null);
      })
      .catch((error) => console.error('Failed to read the recordings move status:', error));
    void refreshGather();

    const offs = [
      safeListen<unknown>('recordings-move-progress', (e) => {
        if (isMoveStatus(e.payload)) setStatus(e.payload);
      }),
      safeListen<unknown>('recordings-move-finished', (e) => {
        setStatus(null);
        setStopping(false);
        const f = isMoveFinished(e.payload) ? e.payload : null;
        setLastFailure(f?.failed[0]?.reason ?? null);
        void refreshGather();
      }),
      safeListen('recordings-gather-blocked', () => void refreshGather()),
      safeListen('recordings-gather-needed', () => void refreshGather()),
    ];
    return () => {
      cancelled = true;
      offs.forEach((off) => off());
    };
  }, [refreshGather]);

  /** Stop between meetings. The one in flight finishes or rolls back. */
  const stop = useCallback(async () => {
    setStopping(true);
    try {
      await invoke('api_cancel_recordings_move');
    } catch (error) {
      setStopping(false);
      toast.error(errorText(error));
    }
  }, []);

  /** "Move them here": gather everything outside the current folder into it. */
  const gatherHere = useCallback(async () => {
    try {
      await invoke('api_gather_recordings');
    } catch (error) {
      toast.error(errorText(error));
      void refreshGather();
    }
  }, [refreshGather]);

  // Recordings still outside the folder, when nothing is moving them.
  const leftBehind = !status && gather && gather.plan.meetings > 0 ? gather.plan.meetings : 0;
  const leftBehindReason = gather?.blockedReason ?? lastFailure;

  return { status, stopping, stop, gatherHere, leftBehind, leftBehindReason, refreshGather };
}
