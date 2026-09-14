'use client';

import { useEffect, useState } from 'react';
import { safeListen } from '@/lib/safe-listen';

// specs/0049 + 0057 §3.1 — the Zoom mute gate already emits `zoom-mute-changed` {muted}
// (src-tauri/src/zoom/mute_monitor.rs EVENT_MUTE_CHANGED); no frontend listened before Plan 2.
export function useMicGate(): boolean {
  const [muted, setMuted] = useState(false);
  // safeListen's return value IS the cleanup function, so it can be returned directly.
  useEffect(() => safeListen<{ muted: boolean }>('zoom-mute-changed', (e) => setMuted(Boolean(e.payload?.muted))), []);
  // mute_monitor only emits on CHANGE and resets its `last_emitted` on stop without emitting
  // `false`, so a session that ended while muted would leave the gate stuck on for the next
  // one. The end of a recording is the authoritative "no longer gated" edge.
  useEffect(() => safeListen('recording-stopped', () => setMuted(false)), []);
  return muted;
}
