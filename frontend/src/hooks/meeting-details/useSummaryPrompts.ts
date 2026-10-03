import { useState, useEffect, useCallback, useRef } from 'react';
import { invoke as invokeTauri } from '@tauri-apps/api/core';
import { toast } from 'sonner';
import { useSidebar } from '@/components/Sidebar/SidebarProvider';
import {
  isPersistableMeetingId,
  type MeetingPromptState,
  type SummaryPrompt,
} from '@/lib/summary-prompts';

const NOT_READY_MESSAGE = "Save isn't available until the meeting has started recording.";

/** Backend errors reject with a user-readable string; fall back for anything else. */
function errorMessage(error: unknown, fallback: string): string {
  if (typeof error === 'string' && error) return error;
  if (error instanceof Error && error.message) return error.message;
  return fallback;
}

/**
 * Saved summary prompts + the per-meeting prompt state (specs/0079).
 *
 * @param meetingId Persistence target. An explicit argument — even `null` — wins over
 *   the sidebar's viewed meeting (the record screen's id is briefly `null` before the
 *   row exists; picks made then are queued and flushed once the id arrives). When
 *   omitted entirely (meeting-details), falls back to `useSidebar().currentMeeting`.
 */
export function useSummaryPrompts(meetingId?: string | null) {
  const [prompts, setPrompts] = useState<SummaryPrompt[]>([]);
  const [state, setState] = useState<MeetingPromptState | null>(null);
  const [loading, setLoading] = useState(true);

  const hasExplicitId = meetingId !== undefined;
  const { currentMeeting } = useSidebar();
  const candidateId = hasExplicitId ? meetingId : currentMeeting?.id;
  const targetMeetingId = isPersistableMeetingId(candidateId) ? candidateId : null;

  // A pick made before the meeting row exists is queued and flushed when the id shows up.
  const pendingSelectRef = useRef<string | null>(null);
  // Once the user picks this mount, a late-resolving state load must not clobber it.
  const userSelectedRef = useRef(false);
  const prevTargetRef = useRef<string | null>(null);
  // Always-current state for callbacks that need has_series without re-creating.
  const stateRef = useRef<MeetingPromptState | null>(null);
  stateRef.current = state;

  const loadPrompts = useCallback(async () => {
    try {
      const list = (await invokeTauri('api_list_summary_prompts')) as SummaryPrompt[];
      setPrompts(list);
    } catch (error) {
      console.error('Failed to load summary prompts:', error);
    }
  }, []);

  useEffect(() => {
    void loadPrompts();
  }, [loadPrompts]);

  // Initial per-meeting state load (guarded against stale resolution).
  useEffect(() => {
    // Switching directly between two different persistable meetings on one hook
    // instance: drop the previous meeting's pick/queue/state. (null -> id keeps
    // the queued pick so it can flush.)
    const prev = prevTargetRef.current;
    prevTargetRef.current = targetMeetingId;
    if (prev && targetMeetingId && prev !== targetMeetingId) {
      userSelectedRef.current = false;
      pendingSelectRef.current = null;
      setState(null);
    }
    if (!targetMeetingId) {
      setState(null);
      setLoading(false);
      return;
    }
    let cancelled = false;
    setLoading(true);
    const load = async () => {
      try {
        const next = (await invokeTauri('api_get_meeting_prompt_state', {
          meetingId: targetMeetingId,
        })) as MeetingPromptState;
        if (cancelled || userSelectedRef.current) return;
        setState(next);
      } catch (error) {
        console.warn('Failed to load meeting prompt state:', error);
      } finally {
        if (!cancelled) setLoading(false);
      }
    };
    void load();
    return () => {
      cancelled = true;
    };
  }, [targetMeetingId]);

  // Explicit refresh (after a write): always applies the result.
  const refreshFor = useCallback(async (id: string) => {
    try {
      const next = (await invokeTauri('api_get_meeting_prompt_state', {
        meetingId: id,
      })) as MeetingPromptState;
      setState(next);
    } catch (error) {
      console.warn('Failed to refresh meeting prompt state:', error);
    }
  }, []);

  const refresh = useCallback(async () => {
    await loadPrompts();
    if (targetMeetingId) await refreshFor(targetMeetingId);
  }, [loadPrompts, refreshFor, targetMeetingId]);

  const persistPick = useCallback(
    async (id: string, promptId: string) => {
      try {
        await invokeTauri('api_set_meeting_summary_prompt', {
          meetingId: id,
          promptId,
          applyToSeries: stateRef.current?.has_series ?? false,
        });
        await refreshFor(id);
      } catch (error) {
        console.error('Failed to save prompt choice:', error);
        toast.error(errorMessage(error, 'Could not save prompt choice'));
      }
    },
    [refreshFor],
  );

  // Flush a queued pick once the meeting row id arrives.
  useEffect(() => {
    if (targetMeetingId && pendingSelectRef.current) {
      const queued = pendingSelectRef.current;
      pendingSelectRef.current = null;
      void persistPick(targetMeetingId, queued);
    }
  }, [targetMeetingId, persistPick]);

  const selectPrompt = useCallback(
    async (promptId: string) => {
      userSelectedRef.current = true;
      if (targetMeetingId) {
        await persistPick(targetMeetingId, promptId);
      } else {
        pendingSelectRef.current = promptId;
      }
    },
    [targetMeetingId, persistPick],
  );

  const saveOneOff = useCallback(
    async (body: string, extractActionItems: boolean) => {
      if (!targetMeetingId) {
        toast.error(NOT_READY_MESSAGE);
        throw new Error(NOT_READY_MESSAGE);
      }
      userSelectedRef.current = true;
      try {
        await invokeTauri('api_set_meeting_custom_prompt', {
          meetingId: targetMeetingId,
          body,
          extractActionItems,
        });
      } catch (error) {
        toast.error(errorMessage(error, 'Could not save the custom prompt'));
        throw error;
      }
      await refreshFor(targetMeetingId);
    },
    [targetMeetingId, refreshFor],
  );

  const clearOneOff = useCallback(async () => {
    if (!targetMeetingId) {
      toast.error(NOT_READY_MESSAGE);
      return;
    }
    userSelectedRef.current = true;
    try {
      await invokeTauri('api_set_meeting_custom_prompt', {
        meetingId: targetMeetingId,
        body: null,
        extractActionItems: true,
      });
    } catch (error) {
      toast.error(errorMessage(error, 'Could not clear the custom prompt'));
      return;
    }
    await refreshFor(targetMeetingId);
  }, [targetMeetingId, refreshFor]);

  const saveFollowup = useCallback(
    async (opts: { name?: string; toLibrary: boolean; toSeries: boolean }) => {
      if (!targetMeetingId) {
        toast.error(NOT_READY_MESSAGE);
        throw new Error(NOT_READY_MESSAGE);
      }
      try {
        const saved = (await invokeTauri('api_save_custom_prompt_followup', {
          meetingId: targetMeetingId,
          name: opts.name ?? null,
          toLibrary: opts.toLibrary,
          toSeries: opts.toSeries,
        })) as SummaryPrompt;
        await loadPrompts();
        await refreshFor(targetMeetingId);
        return saved;
      } catch (error) {
        toast.error(errorMessage(error, 'Could not save the prompt'));
        throw error;
      }
    },
    [targetMeetingId, loadPrompts, refreshFor],
  );

  return { prompts, state, loading, selectPrompt, saveOneOff, clearOneOff, saveFollowup, refresh };
}
