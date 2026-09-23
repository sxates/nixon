'use client';

import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { safeListen } from '@/lib/safe-listen';

/** Where a meeting's audio is in its life (`meetings.audio_state`, specs/0072). */
export type AudioState = 'pending' | 'processed' | 'failed' | 'purged';

/** `api_meeting_audio_status`: what audio the meeting still has, per capability. */
export interface MeetingAudioStatus {
  /** The mixed `audio.mp4`. */
  mix: boolean;
  /** The system channel "Identify speakers" reads (WAV or Opus). */
  channels: boolean;
  /** The channels are stored compressed. */
  compressed: boolean;
  state: AudioState;
}

/** Tooltip on disabled audio actions after the retention policy deleted the audio. */
export const AUDIO_PURGED_TITLE = 'Audio deleted by your retention setting.';
/** Tooltip when the files are missing for some other reason (moved, deleted by hand). */
export const AUDIO_MISSING_TITLE = 'No audio recording is available for this meeting.';
/** Speaker identification failed; the audio is kept for a retry. */
export const AUDIO_FAILED_NOTE =
  "Speaker identification didn't finish. Audio is kept so you can retry.";

/** Can the meeting be (re)transcribed? A mix, or channels Rust mixes on the fly. */
export const canTranscribe = (s: MeetingAudioStatus) => s.mix || s.channels;

/** Why an audio action is unavailable, or null when the audio is there. */
export function audioGoneTitle(s: MeetingAudioStatus | null, need: 'channels' | 'transcribe') {
  if (!s) return null;
  const has = need === 'channels' ? s.channels : canTranscribe(s);
  if (has) return null;
  return s.state === 'purged' ? AUDIO_PURGED_TITLE : AUDIO_MISSING_TITLE;
}

/**
 * The meeting's audio status (specs/0072 W3). Probed on mount, and re-probed whenever
 * Rust reports a new `audio_state` for this meeting (`meeting-audio-state-changed`), so an
 * open meeting page follows a purge or a failed identification without a refetch.
 * null = unknown (pending or failed probe): callers don't gate on unknown.
 */
export function useMeetingAudioStatus(meetingId: string | undefined): MeetingAudioStatus | null {
  const [status, setStatus] = useState<MeetingAudioStatus | null>(null);

  useEffect(() => {
    setStatus(null);
    if (!meetingId) return;
    let cancelled = false;
    const probe = () =>
      invoke<MeetingAudioStatus>('api_meeting_audio_status', { meetingId })
        .then((s) => {
          if (!cancelled) setStatus(s ?? null);
        })
        .catch((error) => {
          console.error('Failed to check the meeting audio status:', error);
        });
    void probe();
    const off = safeListen<{ meetingId?: string }>('meeting-audio-state-changed', (event) => {
      if (event.payload?.meetingId === meetingId) void probe();
    });
    return () => {
      cancelled = true;
      off();
    };
  }, [meetingId]);

  return status;
}
