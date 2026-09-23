import { describe, expect, it } from 'vitest';
import {
  applyRetentionChoice,
  isLoweringRetention,
  keptForNowSentence,
  retentionChoiceDescription,
  retentionChoiceFromPreferences,
  retentionChoiceFromSelectValue,
  retentionChoiceToSelectValue,
  retentionPolicyFromChoice,
  retentionReportToast,
} from '../audio-retention';

// specs/0072 W3 — the single "Delete audio recordings" control maps onto the stored
// `audio_retention` policy, with the legacy pair mirrored exactly as Rust's
// `AudioRetention::to_legacy` does.
describe('retentionChoiceFromPreferences (stored → UI)', () => {
  it('reads audio_retention first, whatever the legacy pair says', () => {
    expect(
      retentionChoiceFromPreferences({
        audio_retention: { mode: 'after_processing' },
        auto_save: false,
        retention_days: 30,
      }),
    ).toBe('once-processed');
    expect(
      retentionChoiceFromPreferences({
        audio_retention: { mode: 'days', days: 7 },
        auto_save: false,
        retention_days: null,
      }),
    ).toBe(7);
    expect(
      retentionChoiceFromPreferences({
        audio_retention: { mode: 'forever' },
        auto_save: true,
        retention_days: 7,
      }),
    ).toBe('never');
  });

  it('falls back to the legacy pair: auto_save=false is Once processed even with a stale day count', () => {
    expect(retentionChoiceFromPreferences({ auto_save: false, retention_days: null })).toBe(
      'once-processed',
    );
    expect(retentionChoiceFromPreferences({ auto_save: false, retention_days: 30 })).toBe(
      'once-processed',
    );
    expect(retentionChoiceFromPreferences({ auto_save: true, retention_days: null })).toBe('never');
    expect(retentionChoiceFromPreferences({ auto_save: true, retention_days: 14 })).toBe(14);
  });

  it('a degenerate day count reads as Never (keeps audio), not a delete', () => {
    expect(retentionChoiceFromPreferences({ auto_save: true, retention_days: 0 })).toBe('never');
    expect(
      retentionChoiceFromPreferences({
        audio_retention: { mode: 'days', days: -3 },
        auto_save: true,
        retention_days: null,
      }),
    ).toBe('never');
  });
});

describe('applyRetentionChoice (UI → stored)', () => {
  const base = { auto_save: true, retention_days: 30, save_folder: '/tmp/rec' };

  it('Once processed stores after_processing and keeps the day count for a later switch back', () => {
    expect(applyRetentionChoice(base, 'once-processed')).toEqual({
      ...base,
      audio_retention: { mode: 'after_processing' },
      auto_save: false,
      retention_days: 30,
    });
  });

  it('Never stores forever and clears the day count', () => {
    expect(applyRetentionChoice({ ...base, auto_save: false }, 'never')).toEqual({
      ...base,
      audio_retention: { mode: 'forever' },
      auto_save: true,
      retention_days: null,
    });
  });

  it('N days stores days:N and mirrors it', () => {
    expect(applyRetentionChoice({ ...base, auto_save: false }, 7)).toEqual({
      ...base,
      audio_retention: { mode: 'days', days: 7 },
      auto_save: true,
      retention_days: 7,
    });
  });

  it('round-trips: read(apply(choice)) === choice', () => {
    for (const choice of ['once-processed', 'never', 7, 30, 90] as const) {
      expect(retentionChoiceFromPreferences(applyRetentionChoice(base, choice))).toBe(choice);
    }
  });

  it('a bad day count becomes forever, never a delete', () => {
    expect(retentionPolicyFromChoice(0)).toEqual({ mode: 'forever' });
    expect(retentionPolicyFromChoice(Number.NaN)).toEqual({ mode: 'forever' });
  });
});

describe('isLoweringRetention', () => {
  it('is true only when audio would be deleted sooner', () => {
    expect(isLoweringRetention(30, 7)).toBe(true);
    expect(isLoweringRetention('never', 90)).toBe(true);
    expect(isLoweringRetention(7, 'once-processed')).toBe(true);
    expect(isLoweringRetention(7, 30)).toBe(false);
    expect(isLoweringRetention('once-processed', 'never')).toBe(false);
    expect(isLoweringRetention(30, 30)).toBe(false);
  });
});

describe('select-value serialization', () => {
  it('serializes and parses each choice', () => {
    expect(retentionChoiceToSelectValue('once-processed')).toBe('once-processed');
    expect(retentionChoiceToSelectValue(30)).toBe('30');
    expect(retentionChoiceFromSelectValue('once-processed')).toBe('once-processed');
    expect(retentionChoiceFromSelectValue('never')).toBe('never');
    expect(retentionChoiceFromSelectValue('30')).toBe(30);
  });

  it('parses junk safely (never deletes data by accident)', () => {
    expect(retentionChoiceFromSelectValue('')).toBe('never');
    expect(retentionChoiceFromSelectValue('immediately')).toBe('never');
    expect(retentionChoiceFromSelectValue('-5')).toBe('never');
  });
});

describe('copy', () => {
  it('no option claims audio goes when the recording stops', () => {
    for (const choice of ['once-processed', 'never', 7, 30] as const) {
      expect(retentionChoiceDescription(choice)).not.toMatch(/as soon as|recording stops/i);
    }
    expect(retentionChoiceDescription('once-processed')).toMatch(
      /transcribed the meeting and identified the speakers, then deleted/,
    );
    expect(retentionChoiceDescription(30)).toMatch(/never while a meeting is still being transcribed/);
  });

  it('the report toast uses the reported numbers and mentions busy meetings', () => {
    expect(retentionReportToast({ meetingsPurged: 23, bytesFreed: 4.1e9, skippedBusy: 2 })).toEqual({
      title: 'Deleted audio from 23 meetings, 4.1 GB freed',
      description: '2 were busy and will be deleted shortly.',
    });
    expect(retentionReportToast({ meetingsPurged: 1, bytesFreed: 5e6, skippedBusy: 0 })).toEqual({
      title: 'Deleted audio from 1 meeting, 5 MB freed',
      description: undefined,
    });
  });

  it('the kept-for-now line appears only when something is kept', () => {
    const base = { meetings: 4, bytes: 1, busy: 0 };
    expect(keptForNowSentence({ ...base, keptPending: 0, keptFailed: 0 })).toBeNull();
    expect(keptForNowSentence({ ...base, keptPending: 3, keptFailed: 0 })).toBe(
      '3 meetings still being processed keep their audio until they finish.',
    );
  });
});
