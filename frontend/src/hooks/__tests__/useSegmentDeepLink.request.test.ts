import { renderHook, act } from '@testing-library/react';
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { useSegmentDeepLink } from '@/hooks/useSegmentDeepLink';

// specs/0061 W4 (task 3) — clicking a speaker looks up their first segment id
// (api_first_segment_for_speaker) and hands it to the transcript deep-link
// machinery from CODE, not from the `?segment=` URL param. `request(id)` seeds
// the same pending-intent state machine the URL param drives, so it reuses the
// existing pagination pump (actively `loadMore`s until the target's page is
// loaded, or gives up once `hasMore` is false) and the existing
// VirtualizedTranscriptView scroll-and-consume path — no new machinery.

const seg = (id: string) => ({ id });

function baseProps(overrides: Record<string, unknown> = {}) {
    return {
        meetingId: 'meet-1',
        segmentParam: null as string | null,
        segments: [seg('seg-1'), seg('seg-2')],
        hasMore: true,
        isLoading: false,
        isLoadingMore: false,
        loadMore: vi.fn(),
        onClearParam: vi.fn(),
        ...overrides,
    };
}

beforeEach(() => {
    vi.useFakeTimers();
});

afterEach(() => {
    vi.useRealTimers();
});

describe('useSegmentDeepLink — request() (specs/0061 W4 task 3)', () => {
    it('pumps loadMore until the requested segment is not yet loaded', () => {
        const props = baseProps();
        const { result, rerender } = renderHook((p) => useSegmentDeepLink(p), {
            initialProps: props,
        });

        act(() => {
            result.current.request('seg_9');
        });
        expect(result.current.pendingSegmentId).toBe('seg_9');

        act(() => {
            vi.advanceTimersByTime(200);
        });
        expect(props.loadMore).toHaveBeenCalledTimes(1);

        // Still not loaded — pumps again as long as hasMore stays true.
        rerender({ ...props, segments: [seg('seg-1'), seg('seg-2'), seg('seg-3')] });
        act(() => {
            vi.advanceTimersByTime(200);
        });
        expect(props.loadMore).toHaveBeenCalledTimes(2);
        expect(props.onClearParam).not.toHaveBeenCalled();
    });

    it('stops pumping (without abandoning) once the requested segment is loaded', () => {
        const props = baseProps();
        const { result, rerender } = renderHook((p) => useSegmentDeepLink(p), {
            initialProps: props,
        });

        act(() => {
            result.current.request('seg_9');
        });
        rerender({ ...props, segments: [seg('seg-1'), seg('seg-2'), seg('seg_9')] });
        act(() => {
            vi.advanceTimersByTime(500);
        });
        expect(props.loadMore).not.toHaveBeenCalled();
        expect(result.current.pendingSegmentId).toBe('seg_9');
    });

    it('gives up once pagination is exhausted (hasMore=false) without the segment ever appearing', () => {
        const props = baseProps({ hasMore: false });
        const { result } = renderHook((p) => useSegmentDeepLink(p), { initialProps: props });

        act(() => {
            result.current.request('seg_9');
        });
        act(() => {
            vi.advanceTimersByTime(500);
        });
        // Exhausted without a match: the pump loop terminates (no infinite loadMore),
        // and the stale intent is abandoned + the URL cleaned, same as a stale
        // search deep-link.
        expect(props.loadMore).not.toHaveBeenCalled();
        expect(result.current.pendingSegmentId).toBeNull();
        expect(props.onClearParam).toHaveBeenCalledTimes(1);
    });
});
