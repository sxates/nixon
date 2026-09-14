import { describe, expect, it } from 'vitest';
import {
  applyRetentionChoice,
  retentionChoiceFromPreferences,
  retentionChoiceFromSelectValue,
  retentionChoiceToSelectValue,
} from '../audio-retention';

// The single "Delete audio recordings" control maps onto the UNCHANGED backend
// pair { auto_save, retention_days }. These pin the mapping in both directions.
describe('retentionChoiceFromPreferences (stored → UI)', () => {
  it('auto_save=false is Immediately, regardless of stored retention', () => {
    expect(
      retentionChoiceFromPreferences({ auto_save: false, retention_days: null }),
    ).toBe('immediately');
    // A leftover retention value must NOT leak through — save-off always wins.
    expect(
      retentionChoiceFromPreferences({ auto_save: false, retention_days: 30 }),
    ).toBe('immediately');
  });

  it('auto_save=true + retention null is Never', () => {
    expect(
      retentionChoiceFromPreferences({ auto_save: true, retention_days: null }),
    ).toBe('never');
  });

  it('auto_save=true + retention N is the N-days choice', () => {
    expect(retentionChoiceFromPreferences({ auto_save: true, retention_days: 7 })).toBe(7);
    expect(retentionChoiceFromPreferences({ auto_save: true, retention_days: 90 })).toBe(90);
    // Custom values stored outside the presets still round-trip truthfully.
    expect(retentionChoiceFromPreferences({ auto_save: true, retention_days: 14 })).toBe(14);
  });

  it('degenerate stored retention (0 or negative) reads as Never, not Immediately', () => {
    expect(retentionChoiceFromPreferences({ auto_save: true, retention_days: 0 })).toBe('never');
    expect(retentionChoiceFromPreferences({ auto_save: true, retention_days: -3 })).toBe('never');
  });
});

describe('applyRetentionChoice (UI → stored)', () => {
  const base = { auto_save: true, retention_days: 30, save_folder: '/tmp/rec' };

  it('Immediately sets auto_save=false and leaves retention_days untouched', () => {
    expect(applyRetentionChoice(base, 'immediately')).toEqual({
      ...base,
      auto_save: false,
      retention_days: 30, // preserved — restored if the user flips back
    });
  });

  it('Never sets auto_save=true and clears retention_days', () => {
    expect(applyRetentionChoice({ ...base, auto_save: false }, 'never')).toEqual({
      ...base,
      auto_save: true,
      retention_days: null,
    });
  });

  it('N days sets auto_save=true and retention_days=N', () => {
    expect(applyRetentionChoice({ ...base, auto_save: false }, 7)).toEqual({
      ...base,
      auto_save: true,
      retention_days: 7,
    });
  });

  it('preserves unrelated preference fields', () => {
    expect(applyRetentionChoice(base, 90).save_folder).toBe('/tmp/rec');
  });

  it('round-trips: read(apply(choice)) === choice', () => {
    for (const choice of ['immediately', 'never', 7, 30, 90] as const) {
      expect(retentionChoiceFromPreferences(applyRetentionChoice(base, choice))).toBe(choice);
    }
  });
});

describe('select-value serialization', () => {
  it('serializes and parses each choice', () => {
    expect(retentionChoiceToSelectValue('immediately')).toBe('immediately');
    expect(retentionChoiceToSelectValue('never')).toBe('never');
    expect(retentionChoiceToSelectValue(30)).toBe('30');
    expect(retentionChoiceFromSelectValue('immediately')).toBe('immediately');
    expect(retentionChoiceFromSelectValue('never')).toBe('never');
    expect(retentionChoiceFromSelectValue('30')).toBe(30);
  });

  it('parses junk safely (never deletes data by accident)', () => {
    expect(retentionChoiceFromSelectValue('')).toBe('never');
    expect(retentionChoiceFromSelectValue('-5')).toBe('never');
    expect(retentionChoiceFromSelectValue('abc')).toBe('never');
  });
});
