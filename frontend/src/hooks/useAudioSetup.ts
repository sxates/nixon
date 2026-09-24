/**
 * "Who was on the mic?" for one meeting (specs/0078 W3).
 *
 * Wraps `api_get_meeting_audio_setup` / `api_set_meeting_audio_setup`:
 *  - `override` is what the user chose ("auto" = let Nixon detect it);
 *  - `resolved` is the setup the LAST diarization pass actually used, or null until a
 *    pass has run on a build that records it.
 *
 * The setter stores the override AND starts a diarization re-run on the Rust side, so
 * callers hand it to `useDiarization().identifySpeakers(start)` — that keeps the model
 * download, progress and error handling in the one place that already owns them.
 *
 * Re-fetched on `diarization-complete` for this meeting, because that is when
 * `resolved` changes.
 */

import { useCallback, useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { safeListen } from '@/lib/safe-listen';

export type AudioSetupOverride = 'auto' | 'room' | 'call';
export type AudioSetupResolved = 'call' | 'room' | 'hybrid';

export interface MeetingAudioSetup {
  override: AudioSetupOverride;
  resolved: AudioSetupResolved | null;
}

/** `api_set_meeting_audio_setup` result — the same DTO "Identify speakers" returns. */
export interface AudioSetupStartResult {
  started: boolean;
  alreadyRunning: boolean;
}

/** A room (or hybrid) pass clusters the mic, so "You" is no longer structural. */
export function isRoomSetup(resolved: AudioSetupResolved | null | undefined): boolean {
  return resolved === 'room' || resolved === 'hybrid';
}

export interface UseAudioSetupReturn {
  /** null until the first fetch lands (or when it failed). */
  setup: MeetingAudioSetup | null;
  refetch: () => Promise<void>;
  /** Store the override and start a re-run. Rejects with the backend's message. */
  setOverride: (setup: AudioSetupOverride) => Promise<AudioSetupStartResult>;
}

export function useAudioSetup(meetingId: string | undefined): UseAudioSetupReturn {
  const [setup, setSetup] = useState<MeetingAudioSetup | null>(null);

  const refetch = useCallback(async () => {
    if (!meetingId) {
      setSetup(null);
      return;
    }
    try {
      const next = await invoke<MeetingAudioSetup | null>('api_get_meeting_audio_setup', {
        meetingId,
      });
      setSetup(next ?? null);
    } catch (error) {
      // Advisory: without it the menu shows no caption and the owner actions fall back
      // to the "no You yet" rule. Never worth a toast.
      console.warn('Failed to load meeting audio setup:', error);
      setSetup(null);
    }
  }, [meetingId]);

  useEffect(() => {
    setSetup(null);
    void refetch();
  }, [refetch]);

  useEffect(() => {
    if (!meetingId) return;
    return safeListen<{ meeting_id?: string }>('diarization-complete', (event) => {
      if (event.payload.meeting_id === meetingId) void refetch();
    });
  }, [meetingId, refetch]);

  const setOverride = useCallback(
    async (next: AudioSetupOverride) => {
      if (!meetingId) throw new Error('No meeting is open');
      const result = await invoke<AudioSetupStartResult>('api_set_meeting_audio_setup', {
        meetingId,
        setup: next,
      });
      setSetup((prev) => ({ override: next, resolved: prev?.resolved ?? null }));
      return result;
    },
    [meetingId],
  );

  return { setup, refetch, setOverride };
}
