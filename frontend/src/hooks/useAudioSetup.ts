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
 * `resolved` changes on `diarization-complete` for this meeting. The event carries the
 * setup the pass used (`audioSetup`) and whether it was detected or forced
 * (`audioSetupSource`), so we apply it straight from the payload; a payload without it,
 * or one whose source disagrees with the override we hold, falls back to a re-fetch.
 *
 * ONE instance per meeting view: it lives in `useSpeakers` and is passed down as props,
 * so the owner actions, their hint and the "…" submenu all read the same state.
 */

import { useCallback, useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';

export type AudioSetupOverride = 'auto' | 'room' | 'call';
export type AudioSetupResolved = 'call' | 'room' | 'hybrid';

const RESOLVED_VALUES: ReadonlySet<string> = new Set<AudioSetupResolved>(['call', 'room', 'hybrid']);

/** The fields of the `diarization-complete` payload this hook reads. */
export interface DiarizationCompleteSetupPayload {
  meeting_id?: string;
  audioSetup?: AudioSetupResolved;
  audioSetupSource?: 'detected' | 'override';
}

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
  /** Feed a `diarization-complete` payload in. The caller owns the listener (the speakers
   *  controller already has one), so a meeting view subscribes once. */
  applyDiarizationComplete: (payload: DiarizationCompleteSetupPayload) => void;
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

  // Held in a ref so `applyDiarizationComplete` stays stable while comparing against the
  // latest override.
  const setupRef = useRef<MeetingAudioSetup | null>(null);
  useEffect(() => {
    setupRef.current = setup;
  }, [setup]);

  const applyDiarizationComplete = useCallback(
    (payload: DiarizationCompleteSetupPayload) => {
      if (!meetingId || payload.meeting_id !== meetingId) return;
      const resolved = payload.audioSetup;
      if (!resolved || !RESOLVED_VALUES.has(resolved)) {
        void refetch();
        return;
      }
      const held = setupRef.current;
      const override = held?.override ?? 'auto';
      setSetup({ override, resolved });
      // "detected" means the stored override was "auto"; "override" means it wasn't. If
      // that disagrees with what we hold (or we hold nothing yet), re-read the override.
      const agrees =
        payload.audioSetupSource === undefined ||
        (payload.audioSetupSource === 'detected') === (override === 'auto');
      if (!agrees || !held) void refetch();
    },
    [meetingId, refetch],
  );

  const setOverride = useCallback(
    async (next: AudioSetupOverride) => {
      if (!meetingId) throw new Error('No meeting is open');
      let result: AudioSetupStartResult;
      try {
        result = await invoke<AudioSetupStartResult>('api_set_meeting_audio_setup', {
          meetingId,
          setup: next,
        });
      } catch (error) {
        // The override may have been stored before the re-run failed to start; re-read
        // so the menu shows what the backend actually holds.
        void refetch();
        throw error;
      }
      setSetup((prev) => ({ override: next, resolved: prev?.resolved ?? null }));
      return result;
    },
    [meetingId, refetch],
  );

  return { setup, refetch, setOverride, applyDiarizationComplete };
}
