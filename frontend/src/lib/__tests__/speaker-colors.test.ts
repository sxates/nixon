import { describe, it, expect } from 'vitest';
import { speakerColorClass, speakerBgClass } from '@/lib/speaker-colors';

// specs/0057 — speaker/channel colors come from the chart tokens so they flip with the
// theme; CH1 (the owner mic, specs/0046) is always chart-1 and the seven remaining
// channels (CH2..CH8) carry remote speakers. Signatures unchanged.

/** A realistic spread of remote speaker keys: diarization placeholders + display names. */
const REMOTE_KEYS = [
  'Speaker 1',
  'Speaker 2',
  'Speaker 3',
  'Speaker 4',
  'Speaker 5',
  'Speaker 6',
  'Speaker 7',
  'Speaker 8',
  'Alice Chen',
  'Bob Martinez',
  'Carol Nguyen',
  'David Okafor',
  'Eve Lindqvist',
  'Frank Moreau',
  'Grace Park',
  'Hiro Tanaka',
  'Ingrid Solberg',
  'Jamal Washington',
  'Kira Petrova',
  'Liam Connelly',
  'Maya Rodriguez',
  'Noah Fitzgerald',
  'Olivia Bennett',
  'Priya Raman',
  'Quentin Dubois',
  'Rosa Alvarez',
  'Samir Haddad',
  'Tessa Njoroge',
  'Uma Krishnan',
  'Viktor Novak',
];

const REMOTE_CHANNELS = [2, 3, 4, 5, 6, 7, 8];

describe('speaker-colors', () => {
  it('uses only chart tokens', () => {
    for (const k of REMOTE_KEYS) {
      expect(speakerColorClass(k)).toMatch(/^text-chart-[1-8]$/);
      expect(speakerBgClass(k)).toMatch(/^bg-chart-[1-8]$/);
    }
  });

  it('uses every remote channel (chart-2..8) across a realistic speaker set', () => {
    const textClasses = new Set(REMOTE_KEYS.map(speakerColorClass));
    const bgClasses = new Set(REMOTE_KEYS.map(speakerBgClass));
    for (const ch of REMOTE_CHANNELS) {
      expect(textClasses).toContain(`text-chart-${ch}`);
      expect(bgClasses).toContain(`bg-chart-${ch}`);
    }
    expect(textClasses.size).toBe(REMOTE_CHANNELS.length);
    expect(bgClasses.size).toBe(REMOTE_CHANNELS.length);
  });

  it('reserves chart-1 for the local user — no other key maps to it', () => {
    for (const k of REMOTE_KEYS) {
      expect(speakerColorClass(k)).not.toBe('text-chart-1');
      expect(speakerBgClass(k)).not.toBe('bg-chart-1');
    }
  });

  it('pins the local user (CH1 owner mic) to chart-1', () => {
    expect(speakerColorClass('local')).toBe('text-chart-1');
    expect(speakerBgClass('local')).toBe('bg-chart-1');
  });

  it('keeps a key on the same channel for label and dot', () => {
    for (const k of [...REMOTE_KEYS, 'local']) {
      expect(speakerBgClass(k)).toBe(speakerColorClass(k).replace('text-', 'bg-'));
    }
  });

  it('falls back to muted tokens for a null key', () => {
    expect(speakerColorClass(null)).toBe('text-muted-foreground');
    expect(speakerBgClass(null)).toBe('bg-muted');
  });
});
