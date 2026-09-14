import { describe, it, expect } from 'vitest';
import {
  talkTimeBySpeaker,
  shareOfTalk,
  groupSeconds,
  largestRemainderPercents,
} from '@/lib/speaker-talk-time';
import type { Transcript } from '@/types';

const t = (speaker: string | null, a?: number, b?: number, duration?: number): Transcript =>
  ({ id: Math.random().toString(), text: 'x', timestamp: '', speaker, audio_start_time: a, audio_end_time: b, duration } as Transcript);

// specs/0057 §3.5 — share-of-talk is nearly free: the data is already in the transcript rows.
describe('talkTimeBySpeaker', () => {
  it('sums duration, falling back to end-start, per speaker key', () => {
    const m = talkTimeBySpeaker([t('local', 0, 10), t('spk_0', 10, 14, 4), t('local', 20, 25), t(null, 0, 99), t('spk_1')]);
    expect(m.get('local')).toBe(15);
    expect(m.get('spk_0')).toBe(4);
    expect(m.has('spk_1')).toBe(false);
    expect(m.has('')).toBe(false);
  });
  it('shares sum to one and are zero on an empty meeting', () => {
    const s = shareOfTalk(new Map([['local', 30], ['spk_0', 10]]));
    expect(s.get('local')).toBeCloseTo(0.75);
    expect(s.get('spk_0')).toBeCloseTo(0.25);
    expect(shareOfTalk(new Map()).size).toBe(0);
  });
});

// Review round 1, Important 1 — a consolidated group (personId/email) keeps every member
// key live in the transcript rows (only `merge` rewrites `transcripts.speaker`), so its row
// must total all of them.
describe('groupSeconds', () => {
  it('totals every member key of a consolidated group', () => {
    const m = new Map([['local', 30], ['spk_0', 10], ['spk_3', 5]]);
    expect(groupSeconds(m, ['spk_0', 'spk_3'])).toBe(15);
    expect(groupSeconds(m, ['spk_9'])).toBe(0);
    expect(groupSeconds(m, [])).toBe(0);
  });
  it('works on the share map too, so a group share is its members summed', () => {
    const s = shareOfTalk(new Map([['local', 30], ['spk_0', 5], ['spk_3', 5]]));
    expect(groupSeconds(s, ['spk_0', 'spk_3'])).toBeCloseTo(0.25);
  });
});

// Review round 1, minor — percentages are the user-visible arithmetic; they must add to 100.
describe('largestRemainderPercents', () => {
  it('rounds so the row percentages sum to 100', () => {
    const p = largestRemainderPercents([0.125, 0.125, 0.75]);
    expect(p.reduce((a, b) => a + b, 0)).toBe(100);
    expect(p[2]).toBe(75);
    expect([p[0], p[1]].sort()).toEqual([12, 13]);
  });
  it('handles an empty or all-zero meeting', () => {
    expect(largestRemainderPercents([])).toEqual([]);
    expect(largestRemainderPercents([0, 0])).toEqual([0, 0]);
  });
});
