'use client';

import { useRef, useReducer, useState, useMemo, useCallback, startTransition, useEffect } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { ArrowDown, UserPlus, X } from "lucide-react";
import { useAutoScroll } from "@/hooks/useAutoScroll";
import { showJumpToLatest } from "@/lib/auto-scroll";
import { useTranscriptStreaming } from "@/hooks/useTranscriptStreaming";
import { TranscriptEmptyState } from "./TranscriptEmptyState";
import { motion } from "framer-motion";
import { TranscriptSegmentData } from "@/types";
import { speakerBgClass } from "@/lib/speaker-colors";
import { cn } from "@/lib/utils";
import { Button } from "./ui/button";
import { Input } from "./ui/input";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "./ui/dropdown-menu";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "./ui/dialog";
import { TranscriptSegment, type InlineSpeakerAssignment } from "./TranscriptSegmentRow";

export type { InlineSpeakerAssignment };

export interface VirtualizedTranscriptViewProps {
    /** Transcript segments to display */
    segments: TranscriptSegmentData[];
    /** Whether recording is in progress */
    isRecording?: boolean;
    /** Whether recording is paused */
    isPaused?: boolean;
    /** Whether processing/finalizing transcription */
    isProcessing?: boolean;
    /** Whether stopping */
    isStopping?: boolean;
    /** Enable streaming effect for latest segment */
    enableStreaming?: boolean;
    /** Show confidence indicators */
    showConfidence?: boolean;
    /** Completely disable auto-scroll behavior (for meeting details page) */
    disableAutoScroll?: boolean;

    // Pagination props (infinite scroll)
    hasMore?: boolean;
    isLoadingMore?: boolean;
    totalCount?: number;
    loadedCount?: number;
    onLoadMore?: () => void;

    /** specs/0019 WS2.1 — when set, each diarized speaker name becomes an inline
     *  "assign to person" affordance. Only passed when viewing (not recording). */
    assignment?: InlineSpeakerAssignment;

    /** specs/0033 — search deep-link: scroll to this segment id once it appears in
     *  `segments`. Best-effort by design: segment ids regenerate on re-transcription,
     *  so an id that never loads silently no-ops. The PARENT owns the intent's
     *  lifecycle (pagination pump + stale-id abandonment, see useSegmentDeepLink);
     *  this view only performs the scroll and reports completion. */
    scrollToSegmentId?: string;
    /** specs/0033 — whether this view is actually visible. The meeting-details
     *  tabpanels stay mounted display:none, and scrolling a zero-height hidden
     *  subtree silently does nothing — so the scroll waits for visibility. */
    scrollTargetVisible?: boolean;
    /** specs/0033 — the deep-link scroll ran (or gave up after a bounded retry):
     *  the parent consumes the intent and cleans the `?segment=` URL param. */
    onScrollToSegmentDone?: () => void;
    /** specs/0045 WS4 — this is a finished-but-unprocessed deferred recording (audio
     *  on disk, no transcript yet): show the "not processed yet" empty state instead
     *  of the generic welcome copy. Only meaningful when not recording. */
    unprocessed?: boolean;
    /** specs/0045 WS4 — "Process now" action for the unprocessed empty state. When
     *  absent, the button is omitted (e.g. no meetingId to enqueue). */
    onProcessNow?: () => void;
}

// Threshold for enabling virtualization (below this, use simple rendering)
const VIRTUALIZATION_THRESHOLD = 10;

export const VirtualizedTranscriptView: React.FC<VirtualizedTranscriptViewProps> = ({
    segments,
    isRecording = false,
    isPaused = false,
    isProcessing = false,
    isStopping = false,
    enableStreaming = false,
    showConfidence = true,
    disableAutoScroll = false,
    hasMore = false,
    isLoadingMore = false,
    totalCount = 0,
    loadedCount = 0,
    onLoadMore,
    assignment,
    scrollToSegmentId,
    scrollTargetVisible = true,
    onScrollToSegmentDone,
    unprocessed = false,
    onProcessNow,
}) => {
    // Create scroll ref first - shared between virtualizer and auto-scroll hook
    const scrollRef = useRef<HTMLDivElement>(null);
    // Ref for infinite scroll trigger element
    const loadMoreTriggerRef = useRef<HTMLDivElement>(null);

    // Force re-render without flushSync (avoids React warning)
    const [, rerender] = useReducer((x: number) => x + 1, 0);

    // Setup virtualizer for efficient rendering of large lists
    const virtualizer = useVirtualizer({
        count: segments.length,
        getScrollElement: () => scrollRef.current,
        // Initial estimate only — real heights are measured per-row via
        // `virtualizer.measureElement` (data-index ref below), so variable-height
        // rows (avatar + header + multi-line text) never overlap. Bumped from 60
        // to account for the taller avatar/header layout.
        estimateSize: () => 84,
        overscan: 10, // Render extra items above/below viewport
        onChange: () => {
            startTransition(() => {
                rerender();
            });
        },
    });

    // Custom hook for auto-scrolling (supports both virtualized and non-virtualized).
    // specs/0029 WS4.1 — this hook (nextAutoFollow hysteresis) is the SINGLE scroll
    // owner for the transcript; `autoScroll` reflects follow/detached state and
    // `scrollToBottom` re-attaches (used by the "Jump to latest" pill below).
    const { autoScroll, scrollToBottom } = useAutoScroll({
        scrollRef,
        segments,
        isRecording,
        isPaused,
        virtualizer,
        virtualizationThreshold: VIRTUALIZATION_THRESHOLD,
        disableAutoScroll,
    });

    // Streaming text effect hook (typewriter animation for new transcripts)
    const { streamingSegmentId, getDisplayText } = useTranscriptStreaming(
        segments,
        isRecording,
        enableStreaming
    );

    // specs/0039 WS2 — span-level speaker correction. Selection + an optimistic overlay
    // live here (in the view that renders the rows), keyed by transcript id so both
    // survive the windowed list unmounting/re-mounting rows on scroll. The overlay maps
    // a transcript id -> its optimistically-reassigned {speaker,speakerName}; it is
    // applied only at render time (never mutates `segments`, so the virtualizer count,
    // auto-scroll, streaming, and deep-link hooks all keep seeing the real data).
    const spanSelectable = !!assignment && !isRecording;
    const [selectedIds, setSelectedIds] = useState<Set<string>>(new Set());
    const [overlay, setOverlay] = useState<
        Map<string, { speaker: string; speakerName: string }>
    >(new Map());
    const [newSpeakerOpen, setNewSpeakerOpen] = useState(false);
    const [newSpeakerName, setNewSpeakerName] = useState('');
    // Anchor INDEX for shift-click range selection; a ref so the stable toggle handler
    // reads the latest value without re-subscribing every memoized row.
    const anchorIndexRef = useRef<number | null>(null);
    // Latest segments, for index->id range resolution without a stale closure.
    const segmentsRef = useRef(segments);
    segmentsRef.current = segments;

    // Clear selection whenever span selection isn't available (e.g. a recording starts).
    useEffect(() => {
        if (!spanSelectable) {
            setSelectedIds((prev) => (prev.size === 0 ? prev : new Set()));
            anchorIndexRef.current = null;
        }
    }, [spanSelectable]);

    const clearSelection = useCallback(() => {
        setSelectedIds(new Set());
        anchorIndexRef.current = null;
    }, []);

    // Toggle one line, or (shift-click) EXTEND the selection with the contiguous range
    // from the anchor to this line. Shift-click unions the range into the existing set
    // (rather than replacing it), so earlier non-contiguous picks survive — the anchor is
    // left unchanged so the range can be re-extended/shrunk from the same origin. The
    // range is derived from the segments array by index, so it's correct regardless of
    // which rows are currently materialized in the virtual window.
    const handleToggleSelect = useCallback(
        (id: string, index: number, shiftKey: boolean) => {
            setSelectedIds((prev) => {
                if (shiftKey && anchorIndexRef.current !== null) {
                    const lo = Math.min(anchorIndexRef.current, index);
                    const hi = Math.max(anchorIndexRef.current, index);
                    const next = new Set(prev);
                    for (const s of segmentsRef.current.slice(lo, hi + 1)) next.add(s.id);
                    return next;
                }
                const next = new Set(prev);
                if (next.has(id)) next.delete(id);
                else next.add(id);
                anchorIndexRef.current = index;
                return next;
            });
        },
        [],
    );

    // Apply the optimistic overlay for rendering only. Length/order/ids are unchanged.
    const displaySegments = useMemo(() => {
        if (overlay.size === 0) return segments;
        return segments.map((s) => {
            const o = overlay.get(s.id);
            return o ? { ...s, speaker: o.speaker, speakerName: o.speakerName } : s;
        });
    }, [segments, overlay]);

    // specs/0041 WS7.2 — overlay reconciliation. Corrections no longer await the parent's
    // refetch (it runs in the background so the viewport never moves), so a successful
    // write KEEPS its overlay entry until the re-fetched segments actually carry the new
    // speaker key — then the entry is dropped and the server data (authoritative for the
    // display name) takes over. Keyed on the speaker KEY only: the backend may canonicalize
    // the display name (e.g. assign-to-person), and that must win.
    useEffect(() => {
        setOverlay((prev) => {
            if (prev.size === 0) return prev;
            let next: Map<string, { speaker: string; speakerName: string }> | null = null;
            for (const s of segments) {
                const o = (next ?? prev).get(s.id);
                if (o && o.speaker === s.speaker) {
                    if (!next) next = new Map(prev);
                    next.delete(s.id);
                }
            }
            return next ?? prev;
        });
    }, [segments]);

    // Reassign the current selection to an EXISTING speaker: optimistic overlay first,
    // clear the selection, dispatch the bulk write. On success the overlay entries stay
    // until background reconciliation lands (see the effect above). On failure the
    // parent has already toasted; we revert the overlay and restore the selection so
    // the user can retry.
    const reassignSpanTo = useCallback(
        async (speakerKey: string, displayName: string) => {
            if (!assignment) return;
            const ids = Array.from(selectedIds);
            if (ids.length === 0) return;
            setOverlay((prev) => {
                const next = new Map(prev);
                for (const id of ids) next.set(id, { speaker: speakerKey, speakerName: displayName });
                return next;
            });
            clearSelection();
            const ok = await assignment.onReassignSegments(ids, speakerKey);
            if (!ok) {
                setOverlay((prev) => {
                    const next = new Map(prev);
                    for (const id of ids) next.delete(id);
                    return next;
                });
                setSelectedIds(new Set(ids));
            }
        },
        [assignment, selectedIds, clearSelection],
    );

    // specs/0041 WS7.2 — single-line "split" with the same optimistic treatment: the
    // label flips immediately via the overlay, the write is dispatched, and on failure
    // the entry is reverted. No awaited refetch anywhere in the path, so the scroll
    // position is untouched.
    const reassignSegmentOptimistic = useCallback(
        async (transcriptId: string, speakerKey: string): Promise<boolean> => {
            if (!assignment) return false;
            const displayName =
                assignment.speakers.find((s) => s.speakerKey === speakerKey)?.displayName ??
                speakerKey;
            setOverlay((prev) =>
                new Map(prev).set(transcriptId, { speaker: speakerKey, speakerName: displayName }),
            );
            const ok = await assignment.onReassignSegment(transcriptId, speakerKey);
            if (!ok) {
                setOverlay((prev) => {
                    const next = new Map(prev);
                    next.delete(transcriptId);
                    return next;
                });
            }
            return ok;
        },
        [assignment],
    );

    // The assignment the rows see: identical wiring, with the single-line reassign
    // routed through the optimistic overlay above.
    const viewAssignment = useMemo<InlineSpeakerAssignment | undefined>(
        () =>
            assignment
                ? { ...assignment, onReassignSegment: reassignSegmentOptimistic }
                : undefined,
        [assignment, reassignSegmentOptimistic],
    );

    // "New speaker…" → mint the speaker, then reassign the selection to it.
    // REVIEW(0039): non-atomic create+reassign can orphan a manual speaker on partial
    // failure — if onCreateSpeaker succeeds but the reassign fails, an embedding-less
    // manual_<uuid> speaker is left with zero lines. Proper fix is an atomic backend
    // mint+reassign (owner decision) — see morning report.
    const createSpeakerAndReassign = useCallback(
        async (rawName: string) => {
            if (!assignment) return;
            const name = rawName.trim();
            if (!name) return;
            const created = await assignment.onCreateSpeaker(name);
            if (!created) return; // parent toasted; keep the selection for a retry
            await reassignSpanTo(created.speakerKey, created.displayName);
        },
        [assignment, reassignSpanTo],
    );

    const submitNewSpeaker = useCallback(() => {
        const name = newSpeakerName.trim();
        if (!name) return;
        setNewSpeakerOpen(false);
        setNewSpeakerName('');
        void createSpeakerAndReassign(name);
    }, [newSpeakerName, createSpeakerAndReassign]);

    const selectionCount = selectedIds.size;
    const selectionActive = selectionCount > 0;

    // Infinite scroll: IntersectionObserver to trigger loading more
    useEffect(() => {
        if (!onLoadMore || !hasMore || isLoadingMore || isRecording || segments.length === 0) {
            return;
        }

        const triggerElement = loadMoreTriggerRef.current;
        if (!triggerElement) return;

        const observer = new IntersectionObserver(
            (entries) => {
                if (entries[0].isIntersecting && hasMore && !isLoadingMore) {
                    onLoadMore();
                }
            },
            {
                root: null,
                rootMargin: '100px',
                threshold: 0,
            }
        );

        observer.observe(triggerElement);

        return () => observer.disconnect();
    }, [hasMore, isLoadingMore, onLoadMore, isRecording, segments.length]);

    // Scroll-based fallback for fast scrolling
    useEffect(() => {
        if (!onLoadMore || !hasMore || isLoadingMore || isRecording) return;

        const scrollElement = scrollRef.current;
        if (!scrollElement) return;

        let ticking = false;

        const handleScroll = () => {
            if (ticking || isLoadingMore || !hasMore) return;

            ticking = true;
            requestAnimationFrame(() => {
                const { scrollTop, scrollHeight, clientHeight } = scrollElement;
                const scrollBottom = scrollHeight - scrollTop - clientHeight;

                // Trigger load when within 200px of bottom
                if (scrollBottom < 200 && hasMore && !isLoadingMore) {
                    onLoadMore();
                }
                ticking = false;
            });
        };

        scrollElement.addEventListener('scroll', handleScroll, { passive: true });
        return () => scrollElement.removeEventListener('scroll', handleScroll);
    }, [onLoadMore, hasMore, isLoadingMore, isRecording]);

    // Use simple rendering for small lists, virtualization for large lists
    const useVirtualization = segments.length >= VIRTUALIZATION_THRESHOLD;

    // specs/0033 — search deep-link: scroll to the target segment. The parent pump
    // (useSegmentDeepLink) guarantees the segment's page gets loaded; this effect
    // waits for (a) the id to be in `segments` and (b) the view to actually be
    // VISIBLE (the tabpanel is kept mounted display:none, and this child effect
    // flushes before the parent's tab-switch effect — a hidden-subtree scroll would
    // silently no-op). It then retries the element lookup across frames: at first
    // commit the virtualizer may not have materialized the row yet (the
    // ResizeObserver → startTransition re-render lands a beat later), and in the
    // meeting-details layout the PAGE column owns the scroll, so
    // virtualizer.scrollToIndex alone is only a best-effort assist that nudges the
    // row into the virtual window. Completion (success, or bounded give-up so the
    // intent can't yank the viewport later) is reported via onScrollToSegmentDone —
    // the intent is never marked consumed before that.
    const completedScrollRef = useRef<string | null>(null);
    // Transient wash on the deep-link target row: with a short transcript the
    // segment may already be on screen, so the scroll alone gives zero feedback —
    // the flash is what tells the user WHERE the match landed.
    const [flashSegmentId, setFlashSegmentId] = useState<string | null>(null);
    useEffect(() => {
        if (!flashSegmentId) return;
        const timer = setTimeout(() => setFlashSegmentId(null), 2400);
        return () => clearTimeout(timer);
    }, [flashSegmentId]);
    useEffect(() => {
        if (!scrollToSegmentId) {
            // Intent cleared — allow the same id to re-fire on a future deep-link.
            completedScrollRef.current = null;
            return;
        }
        if (!scrollTargetVisible) return;
        if (completedScrollRef.current === scrollToSegmentId) return;
        const index = segments.findIndex((s) => s.id === scrollToSegmentId);
        if (index === -1) return; // page not loaded yet — the parent pump is on it

        let cancelled = false;
        let rafId = 0;
        let attempts = 0;
        const MAX_ATTEMPTS = 60; // ~1s of frames
        const finish = () => {
            completedScrollRef.current = scrollToSegmentId;
            onScrollToSegmentDone?.();
        };
        const attempt = () => {
            if (cancelled) return;
            const el = document.getElementById(`segment-${scrollToSegmentId}`);
            if (el) {
                el.scrollIntoView?.({ block: 'center' });
                setFlashSegmentId(scrollToSegmentId);
                finish();
                return;
            }
            if (useVirtualization) {
                // Bring the row into the virtualizer's window so a later frame can
                // find the element (its own container scroll is a no-op here).
                virtualizer.scrollToIndex(index, { align: 'center' });
            }
            attempts += 1;
            if (attempts < MAX_ATTEMPTS) {
                rafId = requestAnimationFrame(attempt);
            } else {
                // The row never materialized — give up (bounded) and consume, so a
                // late render can't snap the viewport mid-reading.
                finish();
            }
        };
        attempt();
        return () => {
            cancelled = true;
            cancelAnimationFrame(rafId);
        };
    }, [scrollToSegmentId, scrollTargetVisible, segments, useVirtualization, virtualizer, onScrollToSegmentDone]);

    // Whether to surface the detached-state "Jump to latest" pill (live recording only).
    const jumpToLatestVisible = showJumpToLatest({
        isRecording,
        following: autoScroll,
        disableAutoScroll,
        segmentCount: segments.length,
    });

    return (
        // Positioning wrapper only (non-scrolling) — anchors the "Jump to latest" pill
        // over the scroll container without joining the scroll chain.
        <div className="relative flex h-full min-h-0 flex-col">
        {/* tabIndex: the container must be focusable for keyboard scrolling and for the
            follow-lock's keydown unlock listener (ArrowUp/PageUp/Home) to receive events. */}
        <div
            ref={scrollRef}
            role="region"
            aria-label="Meeting transcript"
            tabIndex={0}
            className="flex flex-col h-full overflow-y-auto bg-paper px-4 py-2 focus:outline-none focus-visible:ring-2 focus-visible:ring-ring"
        >
            {/* Content. The redundant in-list "Recording" status bar was removed —
                the recording state + timer already live in the screen header. */}
            <div>
            {segments.length === 0 ? (
                // Empty state
                <TranscriptEmptyState
                    isRecording={isRecording}
                    isPaused={isPaused}
                    unprocessed={unprocessed}
                    onProcessNow={onProcessNow}
                />
            ) : useVirtualization ? (
                // Virtualized rendering for large lists
                <>
                    <div
                        style={{
                            height: virtualizer.getTotalSize(),
                            width: "100%",
                            position: "relative",
                        }}
                    >
                        {virtualizer.getVirtualItems().map((virtualRow) => {
                            const segment = displaySegments[virtualRow.index];
                            const isStreaming = streamingSegmentId === segment.id;

                            return (
                                <div
                                    key={segment.id}
                                    data-index={virtualRow.index}
                                    ref={virtualizer.measureElement}
                                    className={`rounded-md transition-colors duration-1000 ${segment.id === flashSegmentId ? 'bg-brand/15' : ''}`}
                                    style={{
                                        position: "absolute",
                                        top: 0,
                                        left: 0,
                                        width: "100%",
                                        transform: `translateY(${virtualRow.start}px)`,
                                    }}
                                >
                                    <TranscriptSegment
                                        id={segment.id}
                                        index={virtualRow.index}
                                        timestamp={segment.timestamp}
                                        text={getDisplayText(segment)}
                                        confidence={segment.confidence}
                                        isStreaming={isStreaming}
                                        showConfidence={showConfidence}
                                        speaker={segment.speaker}
                                        speakerName={segment.speakerName}
                                        assignment={viewAssignment}
                                        selectable={spanSelectable}
                                        selected={selectedIds.has(segment.id)}
                                        selectionActive={selectionActive}
                                        onToggleSelect={handleToggleSelect}
                                    />
                                </div>
                            );
                        })}
                    </div>

                    {/* Infinite scroll trigger and loading indicator */}
                    {(hasMore || isLoadingMore) && !isRecording && segments.length > 0 && (
                        <div ref={loadMoreTriggerRef} className="flex justify-center items-center py-4 mt-2">
                            {isLoadingMore ? (
                                <div className="flex items-center gap-2 text-muted-foreground">
                                    <div className="w-4 h-4 border-2 border-border border-t-muted-foreground rounded-full animate-spin" />
                                    <span className="text-sm">Loading more...</span>
                                </div>
                            ) : hasMore && totalCount > 0 ? (
                                <span className="text-sm text-muted-foreground">
                                    Showing {loadedCount} of {totalCount} segments
                                </span>
                            ) : null}
                        </div>
                    )}

                    {/* Listening indicator when recording */}
                    {!isStopping && isRecording && !isPaused && !isProcessing && segments.length > 0 && (
                        <motion.div
                            initial={{ opacity: 0 }}
                            animate={{ opacity: 1 }}
                            exit={{ opacity: 0 }}
                            className="flex items-center gap-2 mt-4 text-muted-foreground"
                        >
                            <div className="w-2 h-2 bg-brand rounded-full animate-pulse"></div>
                            <span className="text-sm">Listening...</span>
                        </motion.div>
                    )}
                </>
            ) : (
                // Simple rendering for small lists (better animations)
                <>
                    <div className="space-y-1">
                        {displaySegments.map((segment, index) => {
                            const isStreaming = streamingSegmentId === segment.id;

                            return (
                                <motion.div
                                    key={segment.id}
                                    initial={{ opacity: 0, y: 5 }}
                                    animate={{ opacity: 1, y: 0 }}
                                    transition={{ duration: 0.15 }}
                                    className={`rounded-md transition-colors duration-1000 ${segment.id === flashSegmentId ? 'bg-brand/15' : ''}`}
                                >
                                    <TranscriptSegment
                                        id={segment.id}
                                        index={index}
                                        timestamp={segment.timestamp}
                                        text={getDisplayText(segment)}
                                        confidence={segment.confidence}
                                        isStreaming={isStreaming}
                                        showConfidence={showConfidence}
                                        speaker={segment.speaker}
                                        speakerName={segment.speakerName}
                                        assignment={viewAssignment}
                                        selectable={spanSelectable}
                                        selected={selectedIds.has(segment.id)}
                                        selectionActive={selectionActive}
                                        onToggleSelect={handleToggleSelect}
                                    />
                                </motion.div>
                            );
                        })}
                    </div>

                    {/* Infinite scroll trigger (for small lists that grow) */}
                    {(hasMore || isLoadingMore) && !isRecording && segments.length > 0 && (
                        <div ref={loadMoreTriggerRef} className="flex justify-center items-center py-4 mt-2">
                            {isLoadingMore ? (
                                <div className="flex items-center gap-2 text-muted-foreground">
                                    <div className="w-4 h-4 border-2 border-border border-t-muted-foreground rounded-full animate-spin" />
                                    <span className="text-sm">Loading more...</span>
                                </div>
                            ) : hasMore && totalCount > 0 ? (
                                <span className="text-sm text-muted-foreground">
                                    Showing {loadedCount} of {totalCount} segments
                                </span>
                            ) : null}
                        </div>
                    )}

                    {/* Listening indicator when recording */}
                    {!isStopping && isRecording && !isPaused && !isProcessing && segments.length > 0 && (
                        <motion.div
                            initial={{ opacity: 0 }}
                            animate={{ opacity: 1 }}
                            exit={{ opacity: 0 }}
                            className="flex items-center gap-2 mt-4 text-muted-foreground"
                        >
                            <div className="w-2 h-2 bg-brand rounded-full animate-pulse"></div>
                            <span className="text-sm">Listening...</span>
                        </motion.div>
                    )}
                </>
            )}
            </div>
        </div>

        {/* specs/0039 WS2 — span reassignment action bar. Floats over the transcript
            (like "Jump to latest") so it survives scroll; selection state lives in the
            view, not the DOM. Only in the meeting-details view (spanSelectable). */}
        {spanSelectable && assignment && selectionActive && (
            <div className="absolute bottom-3 left-1/2 z-20 flex -translate-x-1/2 items-center gap-1.5 rounded-[3px] bg-panel border border-border px-2 py-1.5 shadow-lg">
                <span className="pl-1.5 text-xs font-medium text-foreground tabular-nums">
                    {selectionCount} {selectionCount === 1 ? 'line' : 'lines'} selected
                </span>
                <DropdownMenu>
                    <DropdownMenuTrigger asChild>
                        <Button size="sm" className="h-7 px-2.5 text-xs">
                            Reassign to…
                        </Button>
                    </DropdownMenuTrigger>
                    <DropdownMenuContent align="center" side="top" className="w-56">
                        <DropdownMenuLabel className="text-xs font-normal text-muted-foreground">
                            Reassign {selectionCount} {selectionCount === 1 ? 'line' : 'lines'} to…
                        </DropdownMenuLabel>
                        <DropdownMenuSeparator />
                        <div className="max-h-56 overflow-y-auto">
                            {assignment.speakers.map((s) => (
                                <DropdownMenuItem
                                    key={s.speakerKey}
                                    onSelect={() => {
                                        void reassignSpanTo(s.speakerKey, s.displayName);
                                    }}
                                    className="text-sm"
                                >
                                    <span
                                        className={cn(
                                            'mr-2 inline-block h-2.5 w-2.5 flex-shrink-0 rounded-full',
                                            speakerBgClass(s.speakerKey),
                                        )}
                                        aria-hidden
                                    />
                                    <span className="flex-1 truncate">{s.displayName}</span>
                                </DropdownMenuItem>
                            ))}
                        </div>
                        <DropdownMenuSeparator />
                        <DropdownMenuItem
                            onSelect={(e) => {
                                // Keep the selection; open the naming dialog.
                                e.preventDefault();
                                setNewSpeakerName('');
                                setNewSpeakerOpen(true);
                            }}
                            className="text-sm"
                        >
                            <UserPlus size={13} className="mr-2 flex-shrink-0 text-muted-foreground" />
                            New speaker…
                        </DropdownMenuItem>
                    </DropdownMenuContent>
                </DropdownMenu>
                <button
                    type="button"
                    onClick={clearSelection}
                    className="rounded-[3px] p-1 text-muted-foreground transition-colors hover:bg-muted hover:text-foreground"
                    aria-label="Clear selection"
                    title="Clear selection"
                >
                    <X size={14} />
                </button>
            </div>
        )}

        {/* specs/0039 WS2 — name a brand-new speaker for the selected span. */}
        <Dialog open={newSpeakerOpen} onOpenChange={setNewSpeakerOpen}>
            <DialogContent className="sm:max-w-sm">
                <DialogHeader>
                    <DialogTitle>New speaker</DialogTitle>
                    <DialogDescription>
                        Reassign the {selectionCount} selected {selectionCount === 1 ? 'line' : 'lines'} to a
                        brand-new speaker the transcription missed.
                    </DialogDescription>
                </DialogHeader>
                <form
                    onSubmit={(e) => {
                        e.preventDefault();
                        submitNewSpeaker();
                    }}
                    className="space-y-3"
                >
                    <Input
                        autoFocus
                        value={newSpeakerName}
                        onChange={(e) => setNewSpeakerName(e.target.value)}
                        placeholder="Speaker name"
                        aria-label="New speaker name"
                    />
                    <DialogFooter>
                        <Button
                            type="button"
                            variant="outline"
                            onClick={() => setNewSpeakerOpen(false)}
                        >
                            Cancel
                        </Button>
                        <Button type="submit" disabled={!newSpeakerName.trim()}>
                            Create &amp; reassign
                        </Button>
                    </DialogFooter>
                </form>
            </DialogContent>
        </Dialog>

        {/* specs/0029 WS4.1 — detached during a live recording: offer a way back to
            the tail without hijacking the user's reading position. */}
        {jumpToLatestVisible && (
            <button
                type="button"
                onClick={scrollToBottom}
                className="absolute bottom-3 left-1/2 z-10 flex -translate-x-1/2 items-center gap-1.5 rounded-[3px] bg-panel border border-border px-3 py-1.5 text-xs font-medium text-foreground shadow-md transition-colors hover:bg-muted"
            >
                <ArrowDown className="h-3.5 w-3.5" />
                Jump to latest
            </button>
        )}
        </div>
    );
};
