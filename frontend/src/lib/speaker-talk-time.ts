import type { Transcript } from '@/types';

/** Seconds of speech per speaker key (specs/0057 §3.5). Rows without timing are ignored. */
export function talkTimeBySpeaker(transcripts: Transcript[]): Map<string, number> {
  const out = new Map<string, number>();
  for (const t of transcripts) {
    const key = t.speaker;
    if (!key) continue;
    const d =
      typeof t.duration === 'number'
        ? t.duration
        : typeof t.audio_start_time === 'number' &&
            typeof t.audio_end_time === 'number'
          ? t.audio_end_time - t.audio_start_time
          : null;
    if (d === null || !(d > 0)) continue;
    out.set(key, (out.get(key) ?? 0) + d);
  }
  return out;
}

/** Fractions of the meeting's total talk time, 0..1, summing to 1 (empty when silent). */
export function shareOfTalk(seconds: Map<string, number>): Map<string, number> {
  let total = 0;
  seconds.forEach((v) => (total += v));
  const out = new Map<string, number>();
  if (total <= 0) return out;
  seconds.forEach((v, k) => out.set(k, v / total));
  return out;
}

/** m:ss or h:mm:ss for the TIME column. */
export function formatTalkTime(seconds: number): string {
  const s = Math.max(0, Math.floor(seconds));
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const sec = s % 60;
  return h > 0
    ? `${h}:${String(m).padStart(2, '0')}:${String(sec).padStart(2, '0')}`
    : `${m}:${String(sec).padStart(2, '0')}`;
}

/** Total seconds (or share) across every member key of a consolidated speaker group.
 *  Assigning a speaker to a Person groups it in the legend but does NOT rewrite
 *  `transcripts.speaker` (only merging does), so each member key still holds its own
 *  rows and the group's row must add them up. */
export function groupSeconds(byKey: Map<string, number>, keys: string[]): number {
  let total = 0;
  for (const k of keys) total += byKey.get(k) ?? 0;
  return total;
}

/** Whole percentages for a set of 0..1 shares, allocated by largest remainder so the
 *  displayed column adds to exactly 100 (naive rounding can show 12/12/75 = 99). */
export function largestRemainderPercents(shares: number[]): number[] {
  const scaled = shares.map((s) => Math.max(0, s) * 100);
  const floors = scaled.map((v) => Math.floor(v));
  const target = Math.round(scaled.reduce((a, b) => a + b, 0));
  let remaining = target - floors.reduce((a, b) => a + b, 0);
  const order = scaled
    .map((v, i) => ({ i, rem: v - Math.floor(v) }))
    .sort((a, b) => b.rem - a.rem || a.i - b.i);
  const out = [...floors];
  for (const { i } of order) {
    if (remaining <= 0) break;
    out[i] += 1;
    remaining -= 1;
  }
  return out;
}
