'use client';

import { useEffect, useRef, useState } from 'react';
import { safeListen } from '@/lib/safe-listen';

// specs/0057 §3.2 — the backend emits `recording-level` at ~12.5 Hz
// (audio/live_meter.rs LIVE_METER_EMIT_INTERVAL): `{ rms, peak }` for the MIX plus
// `mic` / `sys` objects for the two clean pre-mix channels. The mixed pair drives the rail
// ladder; the channel pairs drive the record page's CH1 MIC / CH2 SYS needles.
const SILENCE_DECAY_MS = 500;
const PEAK_LATCH_MS = 800;

export interface ChannelLevel {
  rms: number;
  peak: number;
}

export interface RecordingLevel {
  /** Mixed (what the recording hears). */
  rms: number;
  peak: number;
  /** Any channel — or the mix — crossed the clip threshold in the last PEAK_LATCH_MS. */
  peakLatched: boolean;
  mic: ChannelLevel;
  sys: ChannelLevel;
}

interface LevelPayload {
  rms?: number;
  peak?: number;
  mic?: Partial<ChannelLevel>;
  sys?: Partial<ChannelLevel>;
}

const SILENT: ChannelLevel = { rms: 0, peak: 0 };
const IDLE: RecordingLevel = { rms: 0, peak: 0, peakLatched: false, mic: SILENT, sys: SILENT };
const CLIP = 0.98;

const num = (v: unknown) => Number(v) || 0;

/** Parse one payload. A channel object missing from an older emitter falls back to the mix. */
export function parseLevelPayload(payload: LevelPayload | undefined): Omit<RecordingLevel, 'peakLatched'> {
  const rms = num(payload?.rms);
  const peak = num(payload?.peak);
  const channel = (c: Partial<ChannelLevel> | undefined): ChannelLevel =>
    c ? { rms: num(c.rms), peak: num(c.peak) } : { rms, peak };
  return { rms, peak, mic: channel(payload?.mic), sys: channel(payload?.sys) };
}

export function useRecordingLevel(enabled: boolean): RecordingLevel {
  const [level, setLevel] = useState<RecordingLevel>(IDLE);
  const decayTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const latchTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    if (!enabled) {
      setLevel(IDLE);
      return;
    }
    let disposed = false;
    // safeListen returns a synchronous, idempotent cleanup (see lib/safe-listen.ts) —
    // there is no promise to await here.
    const dispose = safeListen<LevelPayload>('recording-level', (event) => {
      if (disposed) return;
      const next = parseLevelPayload(event.payload);
      const clipped = Math.max(next.peak, next.mic.peak, next.sys.peak) > CLIP;
      setLevel((prev) => ({ ...next, peakLatched: prev.peakLatched || clipped }));
      if (clipped) {
        if (latchTimer.current) clearTimeout(latchTimer.current);
        latchTimer.current = setTimeout(() => setLevel((p) => ({ ...p, peakLatched: false })), PEAK_LATCH_MS);
      }
      if (decayTimer.current) clearTimeout(decayTimer.current);
      decayTimer.current = setTimeout(
        () => setLevel((p) => ({ ...p, rms: 0, peak: 0, mic: SILENT, sys: SILENT })),
        SILENCE_DECAY_MS,
      );
    });
    return () => {
      disposed = true;
      dispose();
      if (decayTimer.current) clearTimeout(decayTimer.current);
      if (latchTimer.current) clearTimeout(latchTimer.current);
    };
  }, [enabled]);

  return level;
}
