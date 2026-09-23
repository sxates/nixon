import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { createSummaryPoller } from '@/lib/summary-polling';

describe('createSummaryPoller', () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  // Measured 2026-09-23: the meeting page's auto-summary started polling a meeting, then the
  // deferred backlog started its own summary of the same meeting three seconds later. The
  // second start REPLACED the first poll, so the page's callback never fired again and the
  // page said "writing your summary" forever over a summary that had finished.
  it('a second start for the same meeting does not orphan the first subscriber', async () => {
    const fetchSummary = vi.fn().mockResolvedValue({ status: 'completed' });
    const poller = createSummaryPoller(fetchSummary, 5000);
    const page = vi.fn();
    const backlog = vi.fn();

    poller.start('m1', page);
    poller.start('m1', backlog);
    await vi.advanceTimersByTimeAsync(5000);

    expect(page).toHaveBeenCalledWith({ status: 'completed' });
    expect(backlog).toHaveBeenCalledWith({ status: 'completed' });
    // One interval per meeting, not one per subscriber.
    expect(fetchSummary).toHaveBeenCalledTimes(1);
  });

  it('stops polling at a terminal status', async () => {
    const fetchSummary = vi
      .fn()
      .mockResolvedValueOnce({ status: 'processing' })
      .mockResolvedValue({ status: 'completed' });
    const poller = createSummaryPoller(fetchSummary, 5000);
    const cb = vi.fn();

    poller.start('m1', cb);
    await vi.advanceTimersByTimeAsync(15000);

    expect(fetchSummary).toHaveBeenCalledTimes(2);
    expect(cb).toHaveBeenLastCalledWith({ status: 'completed' });
  });

  it('a subscriber added after a poll finished starts a fresh poll', async () => {
    const fetchSummary = vi.fn().mockResolvedValue({ status: 'completed' });
    const poller = createSummaryPoller(fetchSummary, 5000);
    poller.start('m1', vi.fn());
    await vi.advanceTimersByTimeAsync(5000);

    const later = vi.fn();
    poller.start('m1', later);
    await vi.advanceTimersByTimeAsync(5000);
    expect(later).toHaveBeenCalledWith({ status: 'completed' });
  });

  it('stop() silences every subscriber of that meeting only', async () => {
    const fetchSummary = vi.fn().mockResolvedValue({ status: 'processing' });
    const poller = createSummaryPoller(fetchSummary, 5000);
    const a = vi.fn();
    const b = vi.fn();
    poller.start('m1', a);
    poller.start('m2', b);

    poller.stop('m1');
    await vi.advanceTimersByTimeAsync(5000);
    expect(a).not.toHaveBeenCalled();
    expect(b).toHaveBeenCalledWith({ status: 'processing' });
    poller.stopAll();
  });

  it('reports a fetch failure to every subscriber and stops', async () => {
    const fetchSummary = vi.fn().mockRejectedValue(new Error('boom'));
    const poller = createSummaryPoller(fetchSummary, 5000);
    const a = vi.fn();
    const b = vi.fn();
    poller.start('m1', a);
    poller.start('m1', b);
    await vi.advanceTimersByTimeAsync(10000);

    expect(a).toHaveBeenCalledTimes(1);
    expect(a).toHaveBeenCalledWith({ status: 'error', error: 'boom' });
    expect(b).toHaveBeenCalledWith({ status: 'error', error: 'boom' });
  });

  it('times out after maxPolls and tells every subscriber', async () => {
    const fetchSummary = vi.fn().mockResolvedValue({ status: 'processing' });
    const poller = createSummaryPoller(fetchSummary, 5000, 3);
    const cb = vi.fn();
    poller.start('m1', cb);
    await vi.advanceTimersByTimeAsync(20000);

    expect(cb).toHaveBeenLastCalledWith(expect.objectContaining({ status: 'error' }));
    expect(fetchSummary).toHaveBeenCalledTimes(2);
  });

  it('a subscriber joining a running poll resets its timeout', async () => {
    const fetchSummary = vi.fn().mockResolvedValue({ status: 'processing' });
    const poller = createSummaryPoller(fetchSummary, 5000, 3);
    const cb = vi.fn();
    poller.start('m1', cb);
    await vi.advanceTimersByTimeAsync(10000); // two polls in — one short of the timeout
    poller.start('m1', vi.fn());
    await vi.advanceTimersByTimeAsync(10000);

    expect(cb).not.toHaveBeenCalledWith(expect.objectContaining({ status: 'error' }));
    poller.stopAll();
  });
});
