import { describe, it, expect, beforeEach, vi } from 'vitest';
import { renderHook, waitFor } from '@testing-library/react';

// Review round 1, Important 2 — the channel strip's share-of-talk must be computed over the
// WHOLE meeting, not the paginated buffer (DEFAULT_PAGE_SIZE 100), or a long meeting shows
// first-page numbers that drift as the user scrolls. This hook fetches every row.

const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }));

import { useMeetingTalkTime } from '@/hooks/meeting-details/useMeetingTalkTime';

const row = (speaker: string, duration: number) => ({
  id: `${speaker}-${duration}-${Math.random()}`,
  text: 'x',
  timestamp: '',
  speaker,
  duration,
});

describe('useMeetingTalkTime', () => {
  beforeEach(() => invokeMock.mockReset());

  it('totals talk time across every page, not just the first', async () => {
    // Page 1 is the count probe (limit 1); the second call asks for all `total_count` rows.
    invokeMock.mockImplementation(
      async (_cmd: string, args?: { limit?: number }) =>
        args?.limit === 1
          ? { transcripts: [row('local', 10)], total_count: 150, has_more: true }
          : {
              transcripts: [
                ...Array.from({ length: 100 }, () => row('local', 1)),
                ...Array.from({ length: 50 }, () => row('spk_0', 2)),
              ],
              total_count: 150,
              has_more: false,
            },
    );

    const { result } = renderHook(() => useMeetingTalkTime('m1'));
    await waitFor(() => expect(result.current.loading).toBe(false));

    expect(result.current.seconds.get('local')).toBe(100);
    expect(result.current.seconds.get('spk_0')).toBe(100);
    expect(invokeMock).toHaveBeenCalledWith('api_get_meeting_transcripts', {
      meetingId: 'm1',
      limit: 150,
      offset: 0,
    });
  });

  it('is a no-op without a meeting id', async () => {
    const { result } = renderHook(() => useMeetingTalkTime(undefined));
    expect(result.current.loading).toBe(false);
    expect(result.current.seconds.size).toBe(0);
    expect(invokeMock).not.toHaveBeenCalled();
  });

  it('re-fetches when the refresh key changes', async () => {
    invokeMock.mockResolvedValue({ transcripts: [], total_count: 0, has_more: false });
    const { result, rerender } = renderHook(
      ({ k }: { k: number }) => useMeetingTalkTime('m1', k),
      { initialProps: { k: 1 } },
    );
    await waitFor(() => expect(result.current.loading).toBe(false));
    const before = invokeMock.mock.calls.length;
    rerender({ k: 2 });
    await waitFor(() => expect(invokeMock.mock.calls.length).toBeGreaterThan(before));
  });

  it('discards a late response from a previous meeting id', async () => {
    // Out-of-order resolution: the FIRST meeting's fetch settles AFTER the second's.
    // Without a request token the stale answer would overwrite the newer meeting's map.
    const gates: Array<(value: unknown) => void> = [];
    invokeMock.mockImplementation(
      (_cmd: string, args?: { meetingId?: string; limit?: number }) => {
        const page =
          args?.meetingId === 'm1'
            ? { transcripts: [row('old', 99)], total_count: 1, has_more: false }
            : { transcripts: [row('new', 7)], total_count: 1, has_more: false };
        if (args?.meetingId === 'm1') {
          return new Promise((resolve) => gates.push(() => resolve(page)));
        }
        return Promise.resolve(page);
      },
    );

    const { result, rerender } = renderHook(
      ({ id }: { id: string }) => useMeetingTalkTime(id),
      { initialProps: { id: 'm1' } },
    );
    // m1's fetch is parked; switch to m2, which answers immediately.
    rerender({ id: 'm2' });
    await waitFor(() => expect(result.current.seconds.get('new')).toBe(7));

    // Now let m1 answer — late. It must NOT clobber m2's map.
    gates.forEach((release) => release(undefined));
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(result.current.seconds.get('old')).toBeUndefined();
    expect(result.current.seconds.get('new')).toBe(7);
  });

  it('keeps the last good map while a refetch is in flight and flags it stale', async () => {
    let release: () => void = () => {};
    const parked = new Promise<void>((resolve) => {
      release = resolve;
    });
    let call = 0;
    const page = { transcripts: [row('local', 42)], total_count: 1, has_more: false };
    const probe = { transcripts: [], total_count: 1, has_more: false };
    invokeMock.mockImplementation(async () => {
      call += 1;
      // Calls 1/2 = first load (probe, full). Call 3 = the refetch's probe — parked so the
      // refetch stays in flight while we assert the previous map is still served.
      if (call === 3) await parked;
      return call % 2 === 1 ? probe : page;
    });

    const { result, rerender } = renderHook(
      ({ k }: { k: number }) => useMeetingTalkTime('m1', k),
      { initialProps: { k: 1 } },
    );
    try {
      await waitFor(() => expect(result.current.seconds.get('local')).toBe(42));
      expect(result.current.stale).toBe(false);

      rerender({ k: 2 });
      await waitFor(() => expect(result.current.stale).toBe(true));
      // The previous map is still served rather than dipping to empty.
      expect(result.current.seconds.get('local')).toBe(42);
    } finally {
      release();
    }
    await waitFor(() => expect(result.current.stale).toBe(false));
    expect(result.current.seconds.get('local')).toBe(42);
  });
});
