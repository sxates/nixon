import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { safeListen } from '@/lib/safe-listen';

/**
 * Whether a speaker-identification pass is running for this meeting — for surfaces that
 * only need to know THAT, not its progress. `useDiarization` owns the full state (stage,
 * percentage, the model download and its toast) for the transcript tab's button; this reads
 * the same backend run registry and events without any of that.
 *
 * The Summary tab uses it to mark a summary written before the speakers were known as
 * preliminary (the backend regenerates it with names once the pass lands).
 */
export function useDiarizationActive(meetingId: string | null | undefined): boolean {
  const [active, setActive] = useState(false);

  useEffect(() => {
    setActive(false);
    if (!meetingId) return;
    let cancelled = false;

    invoke<{ running?: boolean } | null>('api_diarization_status', { meetingId })
      .then((status) => {
        if (!cancelled && status?.running) setActive(true);
      })
      .catch(() => {
        // Advisory only; the live events below still drive the state.
      });

    // Meeting-less progress events are the model download, which doesn't prove a pass.
    const forThisMeeting = (payload: { meeting_id?: string }) => payload?.meeting_id === meetingId;
    const disposers = [
      safeListen<{ meeting_id?: string }>('diarization-progress', (e) => {
        if (forThisMeeting(e.payload)) setActive(true);
      }),
      safeListen<{ meeting_id?: string }>('diarization-complete', (e) => {
        if (forThisMeeting(e.payload)) setActive(false);
      }),
      safeListen<{ meeting_id?: string }>('diarization-error', (e) => {
        if (forThisMeeting(e.payload)) setActive(false);
      }),
    ];

    return () => {
      cancelled = true;
      disposers.forEach((dispose) => dispose());
    };
  }, [meetingId]);

  return active;
}
