import { useCallback, useEffect, useRef, useState } from 'react';

/** Minimal shape of a loaded transcript segment (only the id matters here). */
interface SegmentLike {
    id: string;
}

interface UseSegmentDeepLinkArgs {
    /** The meeting being viewed — a meeting change voids any pending intent. */
    meetingId: string | null;
    /** `?segment=` from the URL — the scroll target of a search deep-link (specs/0033). */
    segmentParam: string | null;
    /** Currently loaded (paginated) transcript segments. */
    segments: SegmentLike[];
    /** Pagination state from usePaginatedTranscripts. */
    hasMore: boolean;
    /** Initial page load in flight (segments empty + hasMore false ≠ exhausted yet). */
    isLoading: boolean;
    isLoadingMore: boolean;
    loadMore: () => void;
    /** Strip `segment` from the URL (router.replace, keeping the other params). */
    onClearParam: () => void;
}

interface UseSegmentDeepLinkReturn {
    /** The segment the transcript view should scroll to, or null when idle. */
    pendingSegmentId: string | null;
    /** Consume the intent: clear the pending state AND the `?segment=` URL param. */
    consume: () => void;
}

/**
 * Delay between pagination pumps. Must exceed loadMore's internal 100 ms debounce:
 * a fast local page can land inside that window, and a silently swallowed call
 * would stall the pump (no state change → the effect never re-runs).
 */
const PUMP_DELAY_MS = 150;

/**
 * specs/0033 — lifecycle of a `?segment=<transcriptId>` search deep-link.
 *
 * Owns the pending scroll intent so the whole chain behaves as one state machine:
 * - Seeds `pendingSegmentId` from the URL param and holds it in STATE, so clearing
 *   the param afterwards can't cancel an in-flight scroll.
 * - Actively pumps `loadMore` until the page containing the target segment is
 *   loaded — the transcript's IntersectionObserver only fetches pages on user
 *   scroll, which would strand any hit past the first page.
 * - When pagination is exhausted without a match, the id is stale (segment ids
 *   regenerate on re-transcription): silently no-op per the spec, but still
 *   consume so the intent can't yank the viewport later and the URL is cleaned.
 * - `consume()` (also called by the transcript view after a successful scroll)
 *   clears the intent AND strips `segment` from the URL via `onClearParam`, so
 *   re-selecting the same search hit is a real param transition (null → X) that
 *   re-fires the chain instead of a silent no-op.
 */
export function useSegmentDeepLink({
    meetingId,
    segmentParam,
    segments,
    hasMore,
    isLoading,
    isLoadingMore,
    loadMore,
    onClearParam,
}: UseSegmentDeepLinkArgs): UseSegmentDeepLinkReturn {
    const [pendingSegmentId, setPendingSegmentId] = useState<string | null>(segmentParam);

    // A new param arrival (null → X after a consume, or X → Y) seeds a fresh intent.
    // The param going away does NOT clear the intent — that's what lets consume()
    // replace the URL without cancelling its own in-flight scroll.
    useEffect(() => {
        if (segmentParam) setPendingSegmentId(segmentParam);
    }, [segmentParam]);

    // Navigating to a different meeting voids any intent left over from the last one.
    const prevMeetingIdRef = useRef(meetingId);
    useEffect(() => {
        if (prevMeetingIdRef.current !== meetingId) {
            prevMeetingIdRef.current = meetingId;
            setPendingSegmentId(segmentParam);
        }
    }, [meetingId, segmentParam]);

    const consume = useCallback(() => {
        setPendingSegmentId(null);
        onClearParam();
    }, [onClearParam]);

    // Pagination pump + stale-id abandonment. Re-runs as pages land (`segments`
    // changes) and as loading flags settle, so it works like a loop without one.
    useEffect(() => {
        if (!pendingSegmentId) return;
        if (segments.some((s) => s.id === pendingSegmentId)) return; // loaded — the view scrolls & consumes
        if (isLoading || isLoadingMore) return; // wait for the in-flight page, then re-check
        if (hasMore) {
            const timer = setTimeout(() => loadMore(), PUMP_DELAY_MS);
            return () => clearTimeout(timer);
        }
        // Exhausted without a match — stale id (re-transcription regenerated the
        // segment ids). Silent no-op per the spec, but consume to clean the URL.
        consume();
    }, [pendingSegmentId, segments, hasMore, isLoading, isLoadingMore, loadMore, consume]);

    return { pendingSegmentId, consume };
}
