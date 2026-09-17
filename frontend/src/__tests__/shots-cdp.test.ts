import { describe, it, expect } from 'vitest';
import { readyPollScript } from '../../scripts/shots/lib/cdp.mjs';
import { summarize } from '../../scripts/shots/run.mjs';

describe('shots run helpers', () => {
  it('ready script checks the attribute', () => {
    expect(readyPollScript()).toContain("dataset.shotReady === '1'");
  });
  it('summary counts errors and sets exit code', () => {
    const s = summarize([{ file: 'a.png', status: 'ok', ms: 10 }, { file: 'b.png', status: 'error', ms: 5, error: 'x' }]);
    expect(s.ok).toBe(1); expect(s.errors).toBe(1); expect(s.exitCode).toBe(1);
  });
});
