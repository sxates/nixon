import { describe, it, expect } from 'vitest';
import { consolidateSpeakers } from '@/lib/speaker-consolidation';
import type { MeetingSpeaker } from '@/types';

// specs/0019 WS2.4 (note 9) — two speakers assigned to the same person must collapse
// to ONE legend chip. These lock the grouping anchors (personId > email > speakerKey)
// and that ungrouped/unnamed speakers never merge.

const spk = (over: Partial<MeetingSpeaker> & { speakerKey: string }): MeetingSpeaker => ({
  displayName: over.displayName ?? over.speakerKey,
  isLocal: false,
  email: null,
  personId: null,
  ...over,
});

describe('consolidateSpeakers (WS2.4)', () => {
  it('collapses two speakers sharing a personId into one group', () => {
    const groups = consolidateSpeakers([
      spk({ speakerKey: 'spk_0', displayName: 'Priya Patel', personId: 'p3', email: 'priya@acme.io' }),
      spk({ speakerKey: 'spk_1', displayName: 'Priya Patel', personId: 'p3', email: 'priya@acme.io' }),
    ]);
    expect(groups).toHaveLength(1);
    expect(groups[0].keys).toEqual(['spk_0', 'spk_1']);
    expect(groups[0].primary.speakerKey).toBe('spk_0');
    expect(groups[0].members).toHaveLength(2);
  });

  it('groups by email when personId is absent', () => {
    const groups = consolidateSpeakers([
      spk({ speakerKey: 'spk_0', displayName: 'Jordan', email: 'JORDAN@acme.io' }),
      spk({ speakerKey: 'spk_1', displayName: 'Jordan Lee', email: 'jordan@acme.io ' }),
    ]);
    expect(groups).toHaveLength(1);
    expect(groups[0].keys).toEqual(['spk_0', 'spk_1']);
  });

  it('keeps unnamed/ungrouped speakers separate and preserves order', () => {
    const groups = consolidateSpeakers([
      spk({ speakerKey: 'local', displayName: 'You', isLocal: true }),
      spk({ speakerKey: 'spk_0', displayName: 'Speaker 1' }),
      spk({ speakerKey: 'spk_1', displayName: 'Speaker 2' }),
    ]);
    expect(groups.map((g) => g.primary.speakerKey)).toEqual(['local', 'spk_0', 'spk_1']);
    expect(groups.every((g) => g.keys.length === 1)).toBe(true);
  });

  it('does not merge two different people', () => {
    const groups = consolidateSpeakers([
      spk({ speakerKey: 'spk_0', personId: 'p1', email: 'a@x.io' }),
      spk({ speakerKey: 'spk_1', personId: 'p2', email: 'b@x.io' }),
    ]);
    expect(groups).toHaveLength(2);
  });

  it('does not over-collapse: same person beats differing emails', () => {
    // personId wins even if emails differ (stale email on one row).
    const groups = consolidateSpeakers([
      spk({ speakerKey: 'spk_0', personId: 'p1', email: 'old@x.io' }),
      spk({ speakerKey: 'spk_1', personId: 'p1', email: 'new@x.io' }),
    ]);
    expect(groups).toHaveLength(1);
    expect(groups[0].keys).toEqual(['spk_0', 'spk_1']);
  });
});
