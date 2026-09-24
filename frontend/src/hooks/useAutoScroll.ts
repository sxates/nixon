import { useRef, useState, useEffect, useCallback, RefObject } from "react";
import { Virtualizer } from "@tanstack/react-virtual";
import {
    FollowState,
    LOCKED_FOLLOW,
    nextFollowState,
    shouldStickToBottomOnAppend,
} from "@/lib/auto-scroll";

/**
 * Guarded displacement unlock (specs/0041 WS7.1 fix): a scrollbar-thumb drag emits ONLY
 * scroll events (no wheel/touch/key), which the locked reducer ignores — treat an upward
 * scrollTop displacement beyond this epsilon, with scrollHeight UNCHANGED, as user intent.
 * The epsilon absorbs sub-pixel jitter; the scrollHeight guard filters content growth and
 * @tanstack/react-virtual re-measurements (both change scrollHeight and can shift
 * scrollTop without user intent).
 */
const DISPLACEMENT_UNLOCK_EPSILON_PX = 4;

interface UseAutoScrollProps {
    scrollRef: RefObject<HTMLDivElement | null>;
    segments: any[];
    isRecording: boolean;
    isPaused: boolean;
    activeSegmentId?: string;
    virtualizer?: Virtualizer<HTMLDivElement, Element>;
    virtualizationThreshold?: number;
    disableAutoScroll?: boolean; // Completely disable auto-scroll behavior (for meeting details page)
}

interface UseAutoScrollReturn {
    autoScroll: boolean;
    setAutoScroll: (value: boolean) => void;
    scrollToBottom: () => void;
}

/**
 * Custom hook to manage auto-scrolling behavior for transcript
 *
 * Features:
 * - Auto-scrolls to bottom when new content arrives during recording
 * - Pauses auto-scroll when user manually scrolls up
 * - Resumes auto-scroll when user scrolls back to the bottom
 * - specs/0041 WS7.1 — "Jump to latest" (`scrollToBottom`) sets a sticky follow LOCK:
 *   while locked, every new segment scrolls to the bottom unconditionally, and only
 *   user-initiated upward input clears it: wheel / touch drag / ArrowUp / PageUp /
 *   Home, or a guarded upward displacement (scrollTop decreased with scrollHeight
 *   unchanged — the scrollbar-thumb drag, which emits only scroll events). Raw scroll
 *   events otherwise can't: they also fire for programmatic scrolls and content
 *   growth, which is exactly what used to detach follow right after jumping.
 *
 * All follow decisions live in the pure helpers in `lib/auto-scroll.ts`
 * (nextFollowState / shouldStickToBottomOnAppend) so they stay unit-testable.
 *
 * @param segments - Array of transcript segments
 * @param isRecording - Whether recording is in progress
 * @param isPaused - Whether recording is paused
 * @param activeSegmentId - ID of the currently active segment
 * @returns Scroll ref, auto-scroll state, and scroll control functions
 */
export function useAutoScroll({
    scrollRef,
    segments,
    isRecording,
    isPaused,
    activeSegmentId,
    virtualizer,
    virtualizationThreshold = 10,
    disableAutoScroll = false,
}: UseAutoScrollProps): UseAutoScrollReturn {
    const useVirtualization = virtualizer && segments.length >= virtualizationThreshold;
    const [autoScroll, setAutoScrollState] = useState(true);
    // Single source of truth for follow decisions; the `autoScroll` state mirrors
    // `.following` for consumers. A ref so effects/listeners always read the latest
    // value synchronously (no debounce race).
    // Starts LOCKED (see LOCKED_FOLLOW): a fresh live transcript follows until the user
    // scrolls up, not until the first row re-measure.
    const followStateRef = useRef<FollowState>(LOCKED_FOLLOW);

    const applyFollowState = useCallback((next: FollowState) => {
        followStateRef.current = next;
        setAutoScrollState(next.following);
    }, []);

    // Track if we're doing a programmatic scroll
    const isProgrammaticScrollRef = useRef(false);
    // Track previous segment count to detect new segments
    const prevSegmentCountRef = useRef(segments.length);
    // Scroll height as of the last time we looked (programmatic scroll or scroll event).
    // Lets the append effect measure the user's distance from the PREVIOUS bottom —
    // content growth inflates scrollHeight and must not read as "user scrolled up".
    const prevScrollHeightRef = useRef(0);
    // scrollTop as of the last observation (scroll event or programmatic write) — the
    // baseline for the guarded displacement unlock in the scroll handler.
    const lastScrollTopRef = useRef(0);

    /**
     * Scroll to bottom programmatically ("Jump to latest").
     * specs/0041 WS7.1 — also LOCKS follow: from here on, new segments always stick
     * to the bottom until the user explicitly scrolls up.
     */
    const scrollToBottom = useCallback(() => {
        if (scrollRef.current) {
            isProgrammaticScrollRef.current = true;
            scrollRef.current.scrollTop = scrollRef.current.scrollHeight;
            prevScrollHeightRef.current = scrollRef.current.scrollHeight;
            lastScrollTopRef.current = scrollRef.current.scrollTop;
            applyFollowState(nextFollowState(followStateRef.current, { type: 'jump-to-latest' }));

            // Reset the flag after a small delay to account for scroll event propagation
            setTimeout(() => {
                isProgrammaticScrollRef.current = false;
            }, 50);
        }
    }, [scrollRef, applyFollowState]);

    /** External override (kept for API compatibility): forcing follow off also drops
     *  the lock; forcing it on locks, like every other way back to following. */
    const setAutoScroll = useCallback(
        (value: boolean) => {
            applyFollowState(value ? LOCKED_FOLLOW : { following: false, followLocked: false });
        },
        [applyFollowState],
    );

    // A new recording always starts following, whatever the last one was left at — this
    // view can outlive a recording (owner feedback 2026-09-23).
    const wasRecordingRef = useRef(isRecording);
    useEffect(() => {
        if (isRecording && !wasRecordingRef.current) applyFollowState(LOCKED_FOLLOW);
        wasRecordingRef.current = isRecording;
    }, [isRecording, applyFollowState]);

    // Handle scroll events to detect manual scrolling (hysteresis; can never clear the
    // follow lock — scroll events also fire for programmatic and content-growth scrolls).
    useEffect(() => {
        const container = scrollRef.current;
        if (!container) return;

        let scrollTimeout: ReturnType<typeof setTimeout> | null = null;

        const handleScroll = () => {
            const el = scrollRef.current;
            if (!el) return;

            // Observe synchronously (per event, not debounced) so the displacement
            // baseline is always the PREVIOUS event/write, even mid-drag.
            const prevScrollTop = lastScrollTopRef.current;
            const prevScrollHeight = prevScrollHeightRef.current;
            lastScrollTopRef.current = el.scrollTop;
            prevScrollHeightRef.current = el.scrollHeight;

            // Skip if this is a programmatic scroll (baseline still refreshed above, so
            // the next user gesture is measured from where the jump/append left us).
            if (isProgrammaticScrollRef.current) {
                return;
            }

            // Guarded displacement unlock (specs/0041 WS7.1 fix): while LOCKED, a
            // scrollbar-thumb drag emits only scroll events — which the reducer must
            // keep ignoring in general (they also fire for programmatic scrolls and
            // content growth). But an upward scrollTop displacement with scrollHeight
            // UNCHANGED can only be the user: derive the explicit `user-scroll-up`
            // intent here, synchronously, so the very next append can't yank them.
            if (
                followStateRef.current.followLocked &&
                el.scrollHeight === prevScrollHeight &&
                prevScrollTop - el.scrollTop > DISPLACEMENT_UNLOCK_EPSILON_PX
            ) {
                applyFollowState(
                    nextFollowState(followStateRef.current, { type: 'user-scroll-up' }),
                );
            }

            // Debounce scroll handling to prevent rapid state changes
            if (scrollTimeout) {
                clearTimeout(scrollTimeout);
            }

            scrollTimeout = setTimeout(() => {
                const el = scrollRef.current;
                if (!el) return;
                // specs/0019 WS1.2 — hysteresis: re-enable auto-follow only when the
                // user is essentially AT the bottom (not merely "near" it), so scrolling
                // up to review earlier transcript isn't undone by the next segment.
                const distance = el.scrollHeight - el.scrollTop - el.clientHeight;
                prevScrollHeightRef.current = el.scrollHeight;
                applyFollowState(
                    nextFollowState(followStateRef.current, {
                        type: 'scroll',
                        distanceFromBottomPx: distance,
                    }),
                );
            }, 100);
        };

        container.addEventListener("scroll", handleScroll, { passive: true });

        return () => {
            container.removeEventListener("scroll", handleScroll);
            if (scrollTimeout) {
                clearTimeout(scrollTimeout);
            }
        };
    }, [scrollRef, applyFollowState]);

    // specs/0041 WS7.1 — user-INTENT listeners. Only these can clear the follow lock:
    // wheel up, an upward touch drag, or an upward navigation key. They fire exclusively
    // for real user input, unlike scroll events. Updates are synchronous (ref first), so
    // an append landing inside the scroll handler's debounce window can't yank the user.
    useEffect(() => {
        const container = scrollRef.current;
        if (!container) return;

        const userScrollUp = () => {
            // Nothing above the viewport → the input can't scroll up; ignore it so a
            // stray wheel tick on a short transcript doesn't strand follow mode off.
            if (container.scrollTop <= 0) return;
            const state = followStateRef.current;
            if (!state.following && !state.followLocked) return;
            applyFollowState(nextFollowState(state, { type: 'user-scroll-up' }));
        };

        let lastTouchY: number | null = null;
        const handleWheel = (e: WheelEvent) => {
            if (e.deltaY < 0) userScrollUp();
        };
        const handleTouchStart = (e: TouchEvent) => {
            lastTouchY = e.touches[0]?.clientY ?? null;
        };
        const handleTouchMove = (e: TouchEvent) => {
            const y = e.touches[0]?.clientY;
            if (y === undefined) return;
            // Finger moving DOWN drags the content down → the viewport scrolls up.
            if (lastTouchY !== null && y > lastTouchY) userScrollUp();
            lastTouchY = y;
        };
        const handleKeyDown = (e: KeyboardEvent) => {
            if (e.key === 'ArrowUp' || e.key === 'PageUp' || e.key === 'Home') {
                userScrollUp();
            }
        };

        container.addEventListener('wheel', handleWheel, { passive: true });
        container.addEventListener('touchstart', handleTouchStart, { passive: true });
        container.addEventListener('touchmove', handleTouchMove, { passive: true });
        container.addEventListener('keydown', handleKeyDown);

        return () => {
            container.removeEventListener('wheel', handleWheel);
            container.removeEventListener('touchstart', handleTouchStart);
            container.removeEventListener('touchmove', handleTouchMove);
            container.removeEventListener('keydown', handleKeyDown);
        };
    }, [scrollRef, applyFollowState]);

    // Auto-scroll to bottom when new segments arrive during recording
    useEffect(() => {
        // EARLY RETURN: If auto-scroll is completely disabled (e.g. meeting details page)
        if (disableAutoScroll) {
            return;
        }

        const segmentCount = segments.length;
        const prevCount = prevSegmentCountRef.current;
        const hasNewSegments = segmentCount > prevCount;

        // Update the ref for next comparison
        prevSegmentCountRef.current = segmentCount;

        if (!hasNewSegments || !isRecording || isPaused || segmentCount === 0) {
            return;
        }

        // Decide via the shared helper. The distance is measured against the PREVIOUS
        // scroll height (this append already grew scrollHeight in the DOM), so it only
        // reflects actual user movement — a race guard for scrollbar drags the
        // debounced scroll handler hasn't classified yet. While follow is LOCKED the
        // helper sticks unconditionally.
        const el = scrollRef.current;
        const distanceFromPreviousBottom = el
            ? Math.max(0, prevScrollHeightRef.current - el.scrollTop - el.clientHeight)
            : 0;
        if (!shouldStickToBottomOnAppend(followStateRef.current, distanceFromPreviousBottom)) {
            return;
        }

        isProgrammaticScrollRef.current = true;

        if (useVirtualization && virtualizer) {
            // Use scrollToOffset with a large value to ensure we're at the bottom
            const totalSize = virtualizer.getTotalSize();
            virtualizer.scrollToOffset(totalSize + 1000, { align: "end" });

            // Also set scrollTop directly as backup after virtualizer updates
            setTimeout(() => {
                if (scrollRef.current) {
                    scrollRef.current.scrollTop = scrollRef.current.scrollHeight;
                    prevScrollHeightRef.current = scrollRef.current.scrollHeight;
                    lastScrollTopRef.current = scrollRef.current.scrollTop;
                }
            }, 50);
        } else if (scrollRef.current) {
            scrollRef.current.scrollTop = scrollRef.current.scrollHeight;
        }
        if (scrollRef.current) {
            prevScrollHeightRef.current = scrollRef.current.scrollHeight;
            lastScrollTopRef.current = scrollRef.current.scrollTop;
        }

        // Reset the flag after a longer delay for virtualization
        setTimeout(() => {
            isProgrammaticScrollRef.current = false;
        }, 150);
    }, [segments.length, isRecording, isPaused, useVirtualization, virtualizer, scrollRef, disableAutoScroll]);

    // Auto-scroll to active segment (when clicking on search results, etc.)
    useEffect(() => {
        if (activeSegmentId) {
            isProgrammaticScrollRef.current = true;

            if (useVirtualization && virtualizer) {
                const index = segments.findIndex((s: any) => s.id === activeSegmentId);
                if (index >= 0) {
                    virtualizer.scrollToIndex(index, { align: "center", behavior: "smooth" });
                }
            } else {
                const element = document.getElementById(`segment-${activeSegmentId}`);
                if (element) {
                    element.scrollIntoView({ behavior: "smooth", block: "center" });
                }
            }

            // Reset the flag after scroll animation completes
            setTimeout(() => {
                isProgrammaticScrollRef.current = false;
            }, 500);
        }
    }, [activeSegmentId, useVirtualization, virtualizer, segments]);

    return {
        autoScroll,
        setAutoScroll,
        scrollToBottom,
    };
}
