/**
 * Live processing-mode state (low-power-mode spec, §§3,5).
 *
 * Subscribes to the backend's `processing-mode-changed` event, which fires:
 *  - at recording start, carrying `meetingId` (possibly null) — the
 *    session's initial mode; and
 *  - on mid-meeting toggles (e.g. battery state flips, or an explicit
 *    override), without a `meetingId`.
 *
 * Whether the session started (or was ever) deferred is Rust's to remember now
 * (specs/0083: `live_toggle`), since Rust decides what the stop does with it.
 *
 * Because the state above is event-only, it is lost on remount / a late mount
 * (navigating away from /record and back, or an auto-started recording that
 * begins while the user is elsewhere). To fix that, on mount we also query
 * `api_get_session_processing_state`: if a session is active we hydrate the
 * live/battery state (without clobbering a fresher event value).
 *
 * It also subscribes to `power-source-changed` so a mid-recording AC/battery
 * transition updates `onBattery` (Finding 4: plugging in mid-deferred-recording
 * must stop the UI claiming "resumes when plugged in"). This is UI-only — it
 * does NOT auto-go-live (manual-only mid-meeting scope).
 */

import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { safeListen } from '@/lib/safe-listen';

interface ProcessingModeChangedPayload {
  meetingId?: string | null;
  liveTranscription: boolean;
  onBattery: boolean;
}

interface PowerSourceChangedPayload {
  onBattery: boolean;
}

interface SessionProcessingState {
  active: boolean;
  liveTranscription: boolean | null;
  onBattery: boolean;
}

interface UseProcessingModeReturn {
  /** Whether live transcription is currently active; null until the first event arrives. */
  liveTranscription: boolean | null;
  /** Whether the machine is currently on battery; null until the first event arrives. */
  onBattery: boolean | null;
}

export function useProcessingMode(): UseProcessingModeReturn {
  const [liveTranscription, setLiveTranscription] = useState<boolean | null>(null);
  const [onBattery, setOnBattery] = useState<boolean | null>(null);

  useEffect(() => {
    return safeListen<ProcessingModeChangedPayload>('processing-mode-changed', (event) => {
      const { liveTranscription: live, onBattery: battery } = event.payload;
      setLiveTranscription(live);
      setOnBattery(battery);
    });
  }, []);

  // Mid-recording power transitions: keep `onBattery` current so the empty-state
  // copy stops showing battery-specific text once the user plugs in (Finding 4).
  useEffect(() => {
    return safeListen<PowerSourceChangedPayload>('power-source-changed', (event) => {
      setOnBattery(event.payload.onBattery);
    });
  }, []);

  // Late-mount / remount hydration (Finding 2): the event stream may have already
  // fired before this hook mounted. Recover the current session state directly.
  useEffect(() => {
    let cancelled = false;
    invoke<SessionProcessingState>('api_get_session_processing_state')
      .then((state) => {
        if (cancelled || !state.active) return;
        // Only fill values the event stream hasn't already set (prev ?? …) so a
        // fresher live event never gets clobbered by this snapshot.
        setLiveTranscription((prev) => (prev === null ? state.liveTranscription : prev));
        setOnBattery((prev) => (prev === null ? state.onBattery : prev));
      })
      .catch((error) => {
        console.warn('Failed to hydrate processing-mode state on mount:', error);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  return { liveTranscription, onBattery };
}
