import { describe, it, expect, vi, afterEach } from 'vitest';
import { readyPollScript, withDeadline } from '../../scripts/shots/lib/cdp.mjs';
import { summarize } from '../../scripts/shots/run.mjs';

describe('shots run helpers', () => {
  it('ready script checks the attribute', () => {
    expect(readyPollScript()).toContain("dataset.shotReady === '1'");
  });
  it('summary counts errors and sets exit code', () => {
    const s = summarize([{ file: 'a.png', status: 'ok', ms: 10 }, { file: 'b.png', status: 'error', ms: 5, error: 'x' }]);
    expect(s.ok).toBe(1); expect(s.errors).toBe(1); expect(s.exitCode).toBe(1);
  });

  describe('withDeadline', () => {
    afterEach(() => { vi.useRealTimers(); });

    it('rejects with a labeled message when the promise never settles', async () => {
      vi.useFakeTimers();
      const never = new Promise(() => {});
      const assertion = expect(withDeadline(never, 1000, 'http://x/shot')).rejects.toThrow(
        'shot deadline 1000 ms exceeded: http://x/shot',
      );
      await vi.advanceTimersByTimeAsync(1000);
      await assertion;
    });

    it('resolves with the value when the promise settles before the deadline', async () => {
      vi.useFakeTimers();
      await expect(withDeadline(Promise.resolve('ok'), 1000, 'label')).resolves.toBe('ok');
    });
  });
});
