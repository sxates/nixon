import { describe, it, expect } from 'vitest';
import { decideAutostart, AUTOSTART_DEBOUNCE_MS } from '@/lib/backlog-autostart';

describe('decideAutostart', () => {
  it('auto-starts when a backlog exists, on AC, and not already processing', () => {
    expect(decideAutostart({ backlogCount: 2, onBattery: false, isProcessing: false })).toBe(true);
  });
  it('never auto-starts on battery (defeats the battery-saving purpose)', () => {
    expect(decideAutostart({ backlogCount: 2, onBattery: true, isProcessing: false })).toBe(false);
  });
  it('does not auto-start with an empty backlog', () => {
    expect(decideAutostart({ backlogCount: 0, onBattery: false, isProcessing: false })).toBe(false);
  });
  it('does not start a second queue while one is running', () => {
    expect(decideAutostart({ backlogCount: 3, onBattery: false, isProcessing: true })).toBe(false);
  });
  it('uses a short blip-absorbing debounce', () => {
    expect(AUTOSTART_DEBOUNCE_MS).toBe(5000);
  });
});
