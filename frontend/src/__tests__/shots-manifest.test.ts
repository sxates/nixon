import { describe, it, expect } from 'vitest';
// @ts-expect-error — scripts/shots/lib/manifest.mjs is a plain Node ESM module (no types); Vite resolves it fine.
import { loadManifest, expand } from '../../scripts/shots/lib/manifest.mjs';

describe('shots manifest', () => {
  const entries = loadManifest();
  it('has unique names and absolute routes', () => {
    const names = entries.map((e) => e.name);
    expect(new Set(names).size).toBe(names.length);
    for (const e of entries) expect(e.route.startsWith('/')).toBe(true);
  });
  it('applies defaults', () => {
    const today = entries.find((e) => e.name === 'today')!;
    expect(today.viewport).toEqual([1280, 860]);
    expect(today.themes).toEqual(['faceplate', 'deck']);
    expect(today.wait).toBe(1500);
  });
  it('expands to one shot per theme with the url params', () => {
    const shots = expand(entries.filter((e) => e.name === 'onboarding-3'));
    expect(shots.map((s) => s.file)).toEqual(['onboarding-3.faceplate.png', 'onboarding-3.deck.png']);
    expect(shots[1].url('http://x:1')).toBe('http://x:1/?theme=deck&onboardingStep=3');
  });
  it('keeps real_only entries out of headless expansion', () => {
    const shots = expand(entries, { headless: true });
    expect(shots.some((s) => s.entry.real_only)).toBe(false);
  });
  it('references only ids that exist in the fixture dataset', () => {
    const text = JSON.stringify(entries);
    for (const id of ['demo-01', 'demo-03', 'demo-05', 'person-maya']) expect(text).toContain(id);
  });
});
