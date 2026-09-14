import React from 'react';
import { render } from '@testing-library/react';
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { VirtualizedTranscriptView } from '@/components/VirtualizedTranscriptView';
import { TooltipProvider } from '@/components/ui/tooltip';
import type { TranscriptSegmentData } from '@/types';

// specs/0033 — the scroll half of the search deep-link chain. The intent
// lifecycle (pagination pump, stale-id abandonment) lives in useSegmentDeepLink
// and is tested there; here we lock the view's contract:
// - never scroll or consume while the view is hidden (the meeting-details
//   tabpanel stays mounted display:none and the child effect flushes before the
//   parent's tab switch);
// - once visible AND the id is loaded, scroll the target row into view and
//   report completion exactly once;
// - an id missing from `segments` is NOT consumed (the parent pump owns it);
// - a cleared-then-reseeded intent for the same id fires again (re-selecting
//   the same ⌘K hit).
// The multi-frame virtualizer retry needs real layout — covered by the manual
// smoke path, not jsdom.

const SEGMENTS: TranscriptSegmentData[] = [
    { id: 'seg-1', timestamp: 0, text: 'hello there' },
    { id: 'seg-2', timestamp: 5, text: 'the target line' },
    { id: 'seg-3', timestamp: 9, text: 'goodbye' },
];

// Record which elements scrollIntoView ran against (vitest spies lose `this`).
let scrolled: Element[] = [];
const originalScrollIntoView = Element.prototype.scrollIntoView;

beforeEach(() => {
    scrolled = [];
    Element.prototype.scrollIntoView = function (this: Element) {
        scrolled.push(this);
    };
});

afterEach(() => {
    Element.prototype.scrollIntoView = originalScrollIntoView;
});

function view(props: Partial<React.ComponentProps<typeof VirtualizedTranscriptView>> = {}) {
    return (
        <TooltipProvider>
            <VirtualizedTranscriptView segments={SEGMENTS} disableAutoScroll={true} {...props} />
        </TooltipProvider>
    );
}

describe('VirtualizedTranscriptView — search deep-link scroll (specs/0033)', () => {
    it('does not scroll or consume while the view is hidden', () => {
        const onDone = vi.fn();
        render(
            view({
                scrollToSegmentId: 'seg-2',
                scrollTargetVisible: false,
                onScrollToSegmentDone: onDone,
            }),
        );
        expect(scrolled).toHaveLength(0);
        expect(onDone).not.toHaveBeenCalled();
    });

    it('scrolls the target row and consumes once the view becomes visible', () => {
        const onDone = vi.fn();
        const { rerender } = render(
            view({
                scrollToSegmentId: 'seg-2',
                scrollTargetVisible: false,
                onScrollToSegmentDone: onDone,
            }),
        );
        rerender(
            view({
                scrollToSegmentId: 'seg-2',
                scrollTargetVisible: true,
                onScrollToSegmentDone: onDone,
            }),
        );
        expect(scrolled).toContain(document.getElementById('segment-seg-2'));
        expect(onDone).toHaveBeenCalledTimes(1);
    });

    it('does not consume an id that is not in the loaded segments (parent pump owns it)', () => {
        const onDone = vi.fn();
        render(
            view({
                scrollToSegmentId: 'seg-999',
                scrollTargetVisible: true,
                onScrollToSegmentDone: onDone,
            }),
        );
        expect(scrolled).toHaveLength(0);
        expect(onDone).not.toHaveBeenCalled();
    });

    it('re-fires for the same id after the intent is cleared and re-seeded', () => {
        const onDone = vi.fn();
        const { rerender } = render(
            view({
                scrollToSegmentId: 'seg-2',
                scrollTargetVisible: true,
                onScrollToSegmentDone: onDone,
            }),
        );
        expect(onDone).toHaveBeenCalledTimes(1);

        // Intent consumed → prop cleared (URL param replaced away)…
        rerender(view({ scrollTargetVisible: true, onScrollToSegmentDone: onDone }));
        // …then the user re-selects the SAME hit: a fresh intent for the same id.
        rerender(
            view({
                scrollToSegmentId: 'seg-2',
                scrollTargetVisible: true,
                onScrollToSegmentDone: onDone,
            }),
        );
        expect(onDone).toHaveBeenCalledTimes(2);
        expect(scrolled.filter((el) => el.id === 'segment-seg-2')).toHaveLength(2);
    });
});
