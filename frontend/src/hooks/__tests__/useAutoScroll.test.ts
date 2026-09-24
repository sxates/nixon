import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { renderHook, act } from '@testing-library/react';
import { useAutoScroll } from '@/hooks/useAutoScroll';

// specs/0041 WS7.1 — "Jump to latest" must LOCK follow mode at the DOM-wiring level:
// scroll events (programmatic / content growth) never detach it; only user-initiated
// upward input (wheel, here) does. These tests drive the hook against a fake scroll
// container with scriptable geometry, complementing the pure-function tests in
// lib/__tests__/auto-scroll.test.ts.

/** A real div whose scroll geometry we control (jsdom has no layout). */
function makeScrollContainer() {
    const el = document.createElement('div');
    const geometry = { scrollTop: 0, scrollHeight: 1000, clientHeight: 300 };
    Object.defineProperty(el, 'scrollTop', {
        get: () => geometry.scrollTop,
        set: (v: number) => {
            geometry.scrollTop = v;
        },
    });
    Object.defineProperty(el, 'scrollHeight', { get: () => geometry.scrollHeight });
    Object.defineProperty(el, 'clientHeight', { get: () => geometry.clientHeight });
    return { el, geometry };
}

const seg = (n: number) => Array.from({ length: n }, (_, i) => ({ id: `s${i}` }));

describe('useAutoScroll — sticky follow lock (WS7.1)', () => {
    beforeEach(() => {
        vi.useFakeTimers();
    });
    afterEach(() => {
        vi.useRealTimers();
    });

    function setup() {
        const { el, geometry } = makeScrollContainer();
        const scrollRef = { current: el };
        const hook = renderHook(
            (props: { segments: { id: string }[] }) =>
                useAutoScroll({
                    scrollRef,
                    segments: props.segments,
                    isRecording: true,
                    isPaused: false,
                }),
            { initialProps: { segments: seg(3) } },
        );
        return { el, geometry, hook };
    }

    /** Fire a (non-programmatic) scroll event and let the 100ms debounce settle. */
    function settleScrollEvent(el: HTMLDivElement) {
        act(() => {
            el.dispatchEvent(new Event('scroll'));
            vi.advanceTimersByTime(150);
        });
    }

    it('locks on Jump-to-latest and survives content growth + scroll events', () => {
        const { el, geometry, hook } = setup();

        // User wheels up to review → detaches (follow starts locked, so only real upward
        // input can do this — a bare scroll event can't).
        geometry.scrollTop = 200; // distance from bottom: 1000 - 200 - 300 = 500
        act(() => {
            el.dispatchEvent(new WheelEvent('wheel', { deltaY: -100 }));
        });
        settleScrollEvent(el);
        expect(hook.result.current.autoScroll).toBe(false);

        // Jump to latest: scrolls to the bottom and LOCKS follow.
        act(() => {
            hook.result.current.scrollToBottom();
            vi.advanceTimersByTime(60); // programmatic-scroll flag clears (50ms)
        });
        expect(hook.result.current.autoScroll).toBe(true);
        expect(geometry.scrollTop).toBe(1000);

        // Content grows under the viewport; a scroll event lands far from the new
        // bottom. Pre-WS7.1 this detached follow immediately — now the lock holds.
        geometry.scrollHeight = 2000; // distance: 2000 - 1000 - 300 = 700
        settleScrollEvent(el);
        expect(hook.result.current.autoScroll).toBe(true);

        // New segments arrive → still follows: scrolls to the (new) bottom.
        act(() => {
            hook.rerender({ segments: seg(4) });
            vi.advanceTimersByTime(200);
        });
        expect(geometry.scrollTop).toBe(2000);
    });

    it('clears the lock only on user-initiated upward scroll (wheel up)', () => {
        const { el, geometry, hook } = setup();

        act(() => {
            hook.result.current.scrollToBottom();
            vi.advanceTimersByTime(60);
        });
        expect(hook.result.current.autoScroll).toBe(true);

        // Wheel DOWN is not an upward intent — lock stays.
        act(() => {
            el.dispatchEvent(new WheelEvent('wheel', { deltaY: 120 }));
        });
        expect(hook.result.current.autoScroll).toBe(true);

        // Wheel UP is: detaches immediately (synchronously — no debounce race).
        act(() => {
            el.dispatchEvent(new WheelEvent('wheel', { deltaY: -120 }));
        });
        expect(hook.result.current.autoScroll).toBe(false);

        // Appends no longer move the viewport.
        const before = geometry.scrollTop;
        geometry.scrollHeight = 2400;
        act(() => {
            hook.rerender({ segments: seg(4) });
            vi.advanceTimersByTime(200);
        });
        expect(geometry.scrollTop).toBe(before);
    });

    it('ignores a wheel-up that cannot scroll (already at the top / unscrollable)', () => {
        const { el, geometry, hook } = setup();
        geometry.scrollTop = 0; // short transcript: nothing above the viewport
        act(() => {
            el.dispatchEvent(new WheelEvent('wheel', { deltaY: -120 }));
        });
        expect(hook.result.current.autoScroll).toBe(true);
    });

    // Guarded displacement unlock: a scrollbar-thumb drag emits ONLY scroll events, so
    // without this path a locked user had no escape (no wheel/touch/key fires, the pill
    // is hidden while following, and every append yanks them back to the bottom).
    it('unlocks on an upward scrollTop displacement with scrollHeight unchanged (scrollbar drag)', () => {
        const { el, geometry, hook } = setup();

        act(() => {
            hook.result.current.scrollToBottom();
            vi.advanceTimersByTime(60); // programmatic-scroll flag clears (50ms)
        });
        expect(hook.result.current.autoScroll).toBe(true);

        // Thumb dragged up: scrollTop decreases, content did NOT grow.
        geometry.scrollTop = 400; // was 1000 after the jump
        settleScrollEvent(el);
        expect(hook.result.current.autoScroll).toBe(false);

        // Appends no longer move the viewport — the lock is gone.
        geometry.scrollHeight = 2000;
        act(() => {
            hook.rerender({ segments: seg(4) });
            vi.advanceTimersByTime(200);
        });
        expect(geometry.scrollTop).toBe(400);
    });

    it('stays locked when scrollTop decreases but scrollHeight ALSO changed (virtualizer re-measure)', () => {
        const { el, geometry, hook } = setup();

        act(() => {
            hook.result.current.scrollToBottom();
            vi.advanceTimersByTime(60);
        });
        expect(hook.result.current.autoScroll).toBe(true);

        // @tanstack/react-virtual re-measured rows above the viewport: scrollTop shifts
        // up WITHOUT user intent, and the total size (scrollHeight) changes with it.
        geometry.scrollHeight = 1400;
        geometry.scrollTop = 940; // decreased from 1000
        settleScrollEvent(el);
        expect(hook.result.current.autoScroll).toBe(true);
    });

    it('stays locked when the displacement lands while the programmatic-scroll flag is set', () => {
        const { el, geometry, hook } = setup();

        act(() => {
            hook.result.current.scrollToBottom();
            // Do NOT advance past 50ms: the jump's own scroll propagation window.
        });
        expect(hook.result.current.autoScroll).toBe(true);

        // A scroll event with a lower scrollTop lands while the flag is still set
        // (the jump's own write settling) — not user intent; the lock must hold.
        geometry.scrollTop = 600;
        act(() => {
            el.dispatchEvent(new Event('scroll'));
            vi.advanceTimersByTime(10);
        });
        expect(hook.result.current.autoScroll).toBe(true);

        // Let the flag clear with no further events: still locked.
        act(() => {
            vi.advanceTimersByTime(200);
        });
        expect(hook.result.current.autoScroll).toBe(true);
    });

    it('a fresh recording follows through a row re-measure (owner feedback 2026-09-23)', () => {
        const { el, geometry, hook } = setup();
        // At the bottom; then the virtualizer re-measures rows: scrollHeight grows under
        // the viewport and a non-programmatic scroll event lands far from the new bottom.
        // This used to detach follow before the user had touched anything.
        geometry.scrollTop = 700; // distance 0
        geometry.scrollHeight = 1600; // distance: 1600 - 700 - 300 = 600
        settleScrollEvent(el);
        expect(hook.result.current.autoScroll).toBe(true);
        act(() => {
            hook.rerender({ segments: seg(4) });
            vi.advanceTimersByTime(200);
        });
        expect(geometry.scrollTop).toBe(1600);
    });

    it('natural follow (no jump) keeps the hysteresis detach behavior', () => {
        const { el, geometry, hook } = setup();

        // Following at the bottom.
        geometry.scrollTop = 700; // distance 0
        settleScrollEvent(el);
        expect(hook.result.current.autoScroll).toBe(true);

        // User scrolls up past the hysteresis threshold → detaches (no lock involved).
        geometry.scrollTop = 500; // distance 200
        settleScrollEvent(el);
        expect(hook.result.current.autoScroll).toBe(false);
    });
});
