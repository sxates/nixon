import { describe, it, expect, beforeEach, vi } from 'vitest';
import { renderHook, act, waitFor } from '@testing-library/react';

// specs/0041 WS7.2 — the refetch contract. Speaker corrections reconcile via
// onRefetchTranscripts (bound to this hook's refetch); it must PRESERVE the pagination
// window instead of resetting to page one — the old reset() emptied the segments array,
// which shrank the virtualizer and clamped scrollTop to 0 (transcript jumped to the top
// on every reassignment).

const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }));

import { usePaginatedTranscripts } from '@/hooks/usePaginatedTranscripts';

const TOTAL = 150; // page size is 100 → two pages
const ALL = Array.from({ length: TOTAL }, (_, i) => ({
  id: `t${i}`,
  meeting_id: 'm1',
  text: `line ${i}`,
  audio_start_time: i,
  audio_end_time: i + 1,
  speaker: 'spk_0',
  speaker_name: 'Speaker 1',
}));

/** The happy-path IPC: metadata + windowed transcript pages over ALL. */
function defaultInvokeImpl(cmd: string, args?: Record<string, unknown>) {
  switch (cmd) {
    case 'api_get_meeting_metadata':
      return Promise.resolve({ id: 'm1', title: 'Weekly sync' });
    case 'api_get_meeting_transcripts': {
      const offset = (args?.offset as number) ?? 0;
      const limit = (args?.limit as number) ?? 100;
      const page = ALL.slice(offset, offset + limit);
      return Promise.resolve({
        transcripts: page,
        has_more: offset + page.length < TOTAL,
        total_count: TOTAL,
      });
    }
    default:
      return Promise.resolve(null);
  }
}

beforeEach(() => {
  vi.clearAllMocks();
  invokeMock.mockImplementation(defaultInvokeImpl);
});

describe('usePaginatedTranscripts.refetch — preserves the pagination window (WS7.2)', () => {
  it('re-fetches the whole loaded window in place: no page-one reset, no loading flip', async () => {
    const { result } = renderHook(() => usePaginatedTranscripts({ meetingId: 'm1' }));

    // Initial load: first page.
    await waitFor(() => expect(result.current.loadedCount).toBe(100));

    // Reader pages down: second page loads (window = 150).
    await act(async () => {
      await result.current.loadMore();
    });
    expect(result.current.loadedCount).toBe(TOTAL);

    // A speaker correction triggers refetch. Hold the response open so we can assert
    // the in-flight state: the already-loaded window must stay mounted (the old reset()
    // emptied it here, which is what clamped scrollTop to 0).
    let release: (() => void) | null = null;
    let requestedArgs: { limit?: number; offset?: number } | null = null;
    invokeMock.mockImplementation((cmd: string, args?: Record<string, unknown>) => {
      if (cmd === 'api_get_meeting_metadata') {
        return Promise.resolve({ id: 'm1', title: 'Weekly sync' });
      }
      if (cmd === 'api_get_meeting_transcripts') {
        requestedArgs = { limit: args?.limit as number, offset: args?.offset as number };
        return new Promise((resolve) => {
          release = () =>
            resolve({
              transcripts: ALL.map((t) => ({
                ...t,
                speaker: 'spk_1',
                speaker_name: 'Priya Patel',
              })),
              has_more: false,
              total_count: TOTAL,
            });
        });
      }
      return Promise.resolve(null);
    });

    let refetchDone: Promise<void> = Promise.resolve();
    act(() => {
      refetchDone = result.current.refetch();
    });

    // Wait for the reconciliation request to be issued (metadata resolves first).
    await waitFor(() => expect(release).not.toBeNull());

    // The reconciliation request covers the FULL loaded window from offset 0 —
    // not a page-one reset.
    expect(requestedArgs).toEqual({ limit: TOTAL, offset: 0 });

    // Mid-refetch: the window is intact and the list is not tearing down.
    expect(result.current.loadedCount).toBe(TOTAL);
    expect(result.current.isLoading).toBe(false);

    await act(async () => {
      release?.();
      await refetchDone;
    });

    // Reconciled in place: same window size, fresh (corrected) rows.
    expect(result.current.loadedCount).toBe(TOTAL);
    expect(result.current.isLoading).toBe(false);
    expect(result.current.transcripts[0].speaker_name).toBe('Priya Patel');
    expect(result.current.segments).toHaveLength(TOTAL);
  });

  it('clears a stale error on a successful refetch — and keeps it while refetches fail', async () => {
    const { result } = renderHook(() => usePaginatedTranscripts({ meetingId: 'm1' }));
    await waitFor(() => expect(result.current.loadedCount).toBe(100));
    expect(result.current.error).toBeNull();

    // A failing refetch surfaces the error, and it must stay visible (the clear
    // happens only on the success path).
    invokeMock.mockImplementation((cmd: string) =>
      cmd === 'api_get_meeting_transcripts'
        ? Promise.reject(new Error('db locked'))
        : defaultInvokeImpl(cmd),
    );
    await act(async () => {
      await result.current.refetch();
    });
    expect(result.current.error).toBe('Failed to load transcripts');

    // The next successful refetch supersedes it. The old refetch went through
    // reset() (which nulled the error); the in-place refetch must clear it too,
    // or corrected rows render under a stale error banner forever.
    invokeMock.mockImplementation(defaultInvokeImpl);
    await act(async () => {
      await result.current.refetch();
    });
    expect(result.current.error).toBeNull();
    expect(result.current.loadedCount).toBe(100);
  });
});
