import { describe, it, expect } from 'vitest';
import { answerAsPlainProse, buildScope, cachedAnswerFor, enterTriggersRun } from '@/lib/ask-ai';

// specs/0035 — pure helpers behind the /ask page. Locks:
// (a) buildScope emits timezone-correct ISO-8601 UTC instants: dateFrom =
//     local midnight at the START of the first day (inclusive), dateTo = local
//     midnight at the start of the day AFTER the last day (EXCLUSIVE);
// (b) enterTriggersRun starts the run for any provider (the pre-send consent
//     gate was removed 2026-07-04 — the configured provider IS the consent),
//     but IME composition-confirm Enter never submits.
//
// TZ note: vitest runs in the machine's timezone, so every expected instant is
// computed with the same local-time `new Date(y, m, d)` math the helper uses
// (never hardcoded strings) — the assertions hold in any TZ.

/** Fixed "now": 2026-06-15 14:30 local time. */
const NOW = new Date(2026, 5, 15, 14, 30, 0);

const ISO_UTC_INSTANT = /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(\.\d+)?Z$/;

describe('buildScope — timezone-correct date bounds (exclusive upper)', () => {
  it("7d = [start of local day (now − 6d), start of tomorrow local)", () => {
    const scope = buildScope({ preset: '7d', now: NOW });
    expect(scope.dateFrom).toBe(new Date(2026, 5, 9).toISOString());
    expect(scope.dateTo).toBe(new Date(2026, 5, 16).toISOString());
  });

  it("30d = [start of local day (now − 29d), start of tomorrow local)", () => {
    const scope = buildScope({ preset: '30d', now: NOW });
    expect(scope.dateFrom).toBe(new Date(2026, 4, 17).toISOString());
    expect(scope.dateTo).toBe(new Date(2026, 5, 16).toISOString());
  });

  it('rolls presets over month boundaries', () => {
    const scope = buildScope({ preset: '7d', now: new Date(2026, 6, 3, 9, 0) }); // Jul 3
    expect(scope.dateFrom).toBe(new Date(2026, 5, 27).toISOString()); // Jun 27
    expect(scope.dateTo).toBe(new Date(2026, 6, 4).toISOString()); // Jul 4
  });

  it('emits full ISO-8601 UTC instants, not day strings', () => {
    const scope = buildScope({ preset: '7d', now: NOW });
    expect(scope.dateFrom).toMatch(ISO_UTC_INSTANT);
    expect(scope.dateTo).toMatch(ISO_UTC_INSTANT);
  });

  it('custom range: from = local midnight of the first day, to = local midnight of the day AFTER the last day', () => {
    const scope = buildScope({
      preset: 'custom',
      customFrom: '2026-03-10',
      customTo: '2026-03-12',
    });
    expect(scope.dateFrom).toBe(new Date(2026, 2, 10).toISOString());
    // Picked through Mar 12 → exclusive bound at start of Mar 13 local.
    expect(scope.dateTo).toBe(new Date(2026, 2, 13).toISOString());
  });

  it('parses custom days as LOCAL calendar days (not UTC midnight)', () => {
    const scope = buildScope({ preset: 'custom', customFrom: '2026-01-01' });
    // Must match the local-time constructor, which differs from
    // new Date('2026-01-01') (UTC midnight) in every non-UTC timezone.
    expect(scope.dateFrom).toBe(new Date(2026, 0, 1).toISOString());
    expect(scope.dateTo).toBeUndefined();
  });

  it('a single-day custom range spans exactly that local day', () => {
    const scope = buildScope({
      preset: 'custom',
      customFrom: '2026-03-10',
      customTo: '2026-03-10',
    });
    expect(scope.dateFrom).toBe(new Date(2026, 2, 10).toISOString());
    expect(scope.dateTo).toBe(new Date(2026, 2, 11).toISOString());
  });

  it('ignores malformed custom dates instead of sending garbage bounds', () => {
    expect(buildScope({ preset: 'custom', customFrom: 'garbage' })).toEqual({});
  });

  it("preset 'all' sets no date bounds; personId passes through independently", () => {
    expect(buildScope({ preset: 'all' })).toEqual({});
    expect(buildScope({ preset: 'all', personId: 'p1' })).toEqual({ personId: 'p1' });
  });
});

// specs/0038 #4 — a saved question's sub-page shows its CACHED answer and only
// re-runs the LLM on Rerun. cachedAnswerFor is the empty-vs-cached decision: the
// page shows the empty ("Not run yet") state exactly when this returns null.
describe('cachedAnswerFor — cached answer vs. never-run empty state', () => {
  it('returns null when the question has never been run', () => {
    expect(cachedAnswerFor({ lastAnswerMarkdown: null, lastSourcesJson: null })).toBeNull();
    expect(cachedAnswerFor({ lastAnswerMarkdown: '', lastSourcesJson: '[]' })).toBeNull();
  });

  it('returns the cached markdown + parsed sources when present', () => {
    const cached = cachedAnswerFor({
      lastAnswerMarkdown: 'We decided to ship. [M1]',
      lastSourcesJson:
        '[{"meetingId":"m1","title":"Standup","createdAt":"2026-07-07T00:00:00Z","cited":true}]',
    });
    expect(cached).not.toBeNull();
    expect(cached?.markdown).toBe('We decided to ship. [M1]');
    expect(cached?.sources).toHaveLength(1);
    expect(cached?.sources[0].meetingId).toBe('m1');
  });

  it('degrades malformed/absent sources JSON to an empty list (answer still shows)', () => {
    expect(cachedAnswerFor({ lastAnswerMarkdown: 'x', lastSourcesJson: null })?.sources).toEqual([]);
    expect(
      cachedAnswerFor({ lastAnswerMarkdown: 'x', lastSourcesJson: 'not json' })?.sources,
    ).toEqual([]);
  });
});

describe('enterTriggersRun — Enter runs the question directly', () => {
  const base = { key: 'Enter', isComposing: false, canAsk: true };

  it('starts a run on a plain Enter', () => {
    expect(enterTriggersRun(base)).toBe(true);
  });

  it('respects canAsk (running / empty question)', () => {
    expect(enterTriggersRun({ ...base, canAsk: false })).toBe(false);
  });

  it('ignores non-Enter keys', () => {
    expect(enterTriggersRun({ ...base, key: 'a' })).toBe(false);
  });

  it('never submits on IME composition-confirm Enter (isComposing)', () => {
    expect(enterTriggersRun({ ...base, isComposing: true })).toBe(false);
  });

  it('never submits on legacy IME keyCode 229', () => {
    expect(enterTriggersRun({ ...base, keyCode: 229 })).toBe(false);
  });
});

// Owner feedback 2026-09-21 — "copy an answer… without all the meeting references
// embedded." The whitespace cases are the point: naive marker removal leaves "October ."
describe('answerAsPlainProse', () => {
  it('drops a marker and the space in front of it before punctuation', () => {
    expect(answerAsPlainProse('We ship rev C in October [M1].')).toBe(
      'We ship rev C in October.',
    );
  });

  it('collapses a run of adjacent markers', () => {
    expect(answerAsPlainProse('Pending the thermal retest [M2][M3].')).toBe(
      'Pending the thermal retest.',
    );
  });

  it('leaves one separating space when words follow the marker', () => {
    expect(answerAsPlainProse('Maya owns it [M1] and Tomas reviews it [M2].')).toBe(
      'Maya owns it and Tomas reviews it.',
    );
  });

  it('handles a marker at the end of a line', () => {
    expect(answerAsPlainProse('- Ship rev C [M1]\n- Retest thermals [M2]')).toBe(
      '- Ship rev C\n- Retest thermals',
    );
  });

  it('keeps the markdown structure intact', () => {
    const md = '## Decisions\n\n- Ship rev C [M1]\n- Hold the retest [M2]\n\n**Owner:** Maya [M1]';
    expect(answerAsPlainProse(md)).toBe(
      '## Decisions\n\n- Ship rev C\n- Hold the retest\n\n**Owner:** Maya',
    );
  });

  it('leaves text with no markers untouched', () => {
    expect(answerAsPlainProse('No citations here.')).toBe('No citations here.');
  });

  it('does not touch bracketed text that is not a marker', () => {
    expect(answerAsPlainProse('See [the doc] and [M1].')).toBe('See [the doc] and.');
  });

  it('is empty for an empty answer', () => {
    expect(answerAsPlainProse('')).toBe('');
  });
});
