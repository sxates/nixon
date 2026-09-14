import { useEffect, useMemo, useRef, useState } from 'react';
import type { Transcript } from '@/types';
import { fetchAllTranscripts } from '@/lib/fetch-all-transcripts';
import { talkTimeBySpeaker } from '@/lib/speaker-talk-time';

const EMPTY: Transcript[] = [];

/**
 * Seconds of speech per speaker key over the WHOLE meeting (specs/0057 §3.5).
 *
 * The transcript panel is paginated (100 rows), so computing share-of-talk from its
 * buffer shows first-page numbers that drift as the user scrolls. This fetches every
 * row once per `meetingId` / `refreshKey` instead. No-op without a meeting id; a fetch
 * failure just leaves the map empty (the strip falls back to the panel's rows).
 *
 * Keep-last-good (specs/0057 Plan 2 residual): a refetch — e.g. after a speaker edit —
 * keeps the previous map and reports `stale: true` instead of dipping to the empty map,
 * so the channel strip never flashes back to the paginated estimate mid-edit. The
 * previous map is dropped only when the `meetingId` itself changes. Responses are keyed
 * by request so an out-of-order resolution (old meeting answering after the new one)
 * cannot overwrite the newer result.
 */
export function useMeetingTalkTime(
  meetingId: string | undefined,
  refreshKey?: unknown,
): { seconds: Map<string, number>; loading: boolean; stale: boolean } {
  const [rows, setRows] = useState<Transcript[]>(EMPTY);
  const [loading, setLoading] = useState(false);
  const [stale, setStale] = useState(false);
  // Monotonic request token: only the newest in-flight fetch may write state.
  const requestRef = useRef(0);
  const lastMeetingIdRef = useRef<string | undefined>(undefined);

  useEffect(() => {
    const request = ++requestRef.current;
    const meetingChanged = lastMeetingIdRef.current !== meetingId;
    lastMeetingIdRef.current = meetingId;

    if (!meetingId) {
      setRows(EMPTY);
      setLoading(false);
      setStale(false);
      return;
    }
    // A different meeting invalidates the previous map; a plain refetch keeps it.
    if (meetingChanged) setRows(EMPTY);
    setLoading(true);
    setStale(!meetingChanged);
    fetchAllTranscripts(meetingId)
      .then((all) => {
        if (requestRef.current !== request) return;
        setRows(all);
        setLoading(false);
        setStale(false);
      })
      .catch((error) => {
        console.error('Failed to fetch transcripts for share-of-talk:', error);
        if (requestRef.current !== request) return;
        setRows(EMPTY);
        setLoading(false);
        setStale(false);
      });
    // Bump the token on unmount too, so a resolve that lands after teardown can't `setRows`
    // on an unmounted hook.
    return () => {
      requestRef.current += 1;
    };
  }, [meetingId, refreshKey]);

  const seconds = useMemo(() => talkTimeBySpeaker(rows), [rows]);
  return { seconds, loading, stale };
}
