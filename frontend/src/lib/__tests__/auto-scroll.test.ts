import { describe, it, expect } from 'vitest';
import {
  LOCKED_FOLLOW,
  nextAutoFollow,
  nextFollowState,
  shouldStickToBottomOnAppend,
  showJumpToLatest,
  AT_BOTTOM_THRESHOLD_PX,
  SCROLLED_UP_THRESHOLD_PX,
  type FollowState,
} from '@/lib/auto-scroll';

// specs/0019 WS1.2 — live-transcript auto-follow hysteresis. The reported bug was the
// view jumping back to the bottom on every new segment while the user reviewed earlier
// transcript; these lock the intended behavior.
describe('nextAutoFollow', () => {
  it('follows when at (or essentially at) the bottom', () => {
    expect(nextAutoFollow(0, false)).toBe(true);
    expect(nextAutoFollow(AT_BOTTOM_THRESHOLD_PX, false)).toBe(true);
  });

  it('detaches once the user scrolls up past the threshold', () => {
    expect(nextAutoFollow(SCROLLED_UP_THRESHOLD_PX + 1, true)).toBe(false);
    expect(nextAutoFollow(500, true)).toBe(false);
  });

  it('stays detached while reviewing, even slightly above the bottom', () => {
    // The old 100px "near bottom" rule re-attached here and caused the jump.
    expect(nextAutoFollow(80, false)).toBe(false);
  });

  it('keeps the current state in the hysteresis band (no flapping)', () => {
    const between = (AT_BOTTOM_THRESHOLD_PX + SCROLLED_UP_THRESHOLD_PX) / 2;
    expect(nextAutoFollow(between, true)).toBe(true);
    expect(nextAutoFollow(between, false)).toBe(false);
  });

  it('re-attaches only on returning to the bottom after being detached', () => {
    expect(nextAutoFollow(SCROLLED_UP_THRESHOLD_PX + 50, false)).toBe(false);
    expect(nextAutoFollow(AT_BOTTOM_THRESHOLD_PX - 1, false)).toBe(true);
  });

  // specs/0029 WS4.1 — the reported live-recording sequence, as the single scroll
  // owner (useAutoScroll) sees it: appends must never re-attach a detached user;
  // only the user returning to the bottom does.
  it('stays detached across live appends until the user returns to the bottom', () => {
    // Following at the bottom.
    let following = nextAutoFollow(0, true);
    expect(following).toBe(true);

    // User scrolls up to read earlier transcript → detach.
    following = nextAutoFollow(400, following);
    expect(following).toBe(false);

    // New segments append while they read: each append grows scrollHeight, so the
    // distance-from-bottom only INCREASES for a stationary viewport. Follow must
    // stay off no matter how many segments arrive.
    for (const distance of [480, 560, 640, 900]) {
      following = nextAutoFollow(distance, following);
      expect(following).toBe(false);
    }

    // User scrolls back down but pauses just above the tail → still detached.
    following = nextAutoFollow(SCROLLED_UP_THRESHOLD_PX + 1, following);
    expect(following).toBe(false);

    // Only landing at the bottom re-attaches.
    following = nextAutoFollow(AT_BOTTOM_THRESHOLD_PX, following);
    expect(following).toBe(true);
  });
});

// specs/0041 WS7.1 — sticky follow intent. "Jump to latest" must LOCK follow mode:
// scroll events (programmatic scrolls, content growth) can never detach it; only a
// user-initiated upward input (wheel/touch/keys) clears it.
describe('nextFollowState (follow lock)', () => {
  const detached: FollowState = { following: false, followLocked: false };
  const following: FollowState = { following: true, followLocked: false };
  const locked: FollowState = { following: true, followLocked: true };

  it('jump-to-latest locks follow mode', () => {
    expect(nextFollowState(detached, { type: 'jump-to-latest' })).toEqual(locked);
  });

  it('while locked, scroll events at ANY distance keep following (content growth)', () => {
    // Content growing under the viewport inflates the distance-from-bottom; the old
    // design re-evaluated the 8px re-attach window here and dropped follow immediately.
    let state = locked;
    for (const distance of [12, 90, 300, 1200]) {
      state = nextFollowState(state, { type: 'scroll', distanceFromBottomPx: distance });
      expect(state).toEqual(locked);
    }
  });

  it('a user upward scroll clears the lock and detaches', () => {
    expect(nextFollowState(locked, { type: 'user-scroll-up' })).toEqual(detached);
  });

  it('after the lock clears, returning to the bottom re-locks (owner feedback 2026-09-23)', () => {
    let state = nextFollowState(locked, { type: 'user-scroll-up' });
    // Reading far up: stays detached.
    state = nextFollowState(state, { type: 'scroll', distanceFromBottomPx: 500 });
    expect(state).toEqual(detached);
    // Returning to the bottom resumes following for good — LOCKED, like Jump to latest.
    state = nextFollowState(state, {
      type: 'scroll',
      distanceFromBottomPx: AT_BOTTOM_THRESHOLD_PX,
    });
    expect(state).toEqual(locked);
    // ...so content growth under the viewport can no longer detach it.
    state = nextFollowState(state, {
      type: 'scroll',
      distanceFromBottomPx: SCROLLED_UP_THRESHOLD_PX + 1,
    });
    expect(state).toEqual(locked);
  });

  it('a live transcript starts locked', () => {
    expect(LOCKED_FOLLOW).toEqual(locked);
  });

  it('a user upward scroll also detaches natural (unlocked) follow', () => {
    expect(nextFollowState(following, { type: 'user-scroll-up' })).toEqual(detached);
  });

  it('the reported sequence: jump → live appends → survives → wheel up → detached', () => {
    // Detached mid-transcript; the user presses "Jump to latest".
    let state = nextFollowState(detached, { type: 'jump-to-latest' });
    expect(state.followLocked).toBe(true);

    // Segments keep appending; every growth/programmatic scroll event lands.
    for (const distance of [84, 20, 168, 5, 92]) {
      state = nextFollowState(state, { type: 'scroll', distanceFromBottomPx: distance });
    }
    expect(state).toEqual(locked); // follows indefinitely

    // Only the user's own upward scroll ends it.
    state = nextFollowState(state, { type: 'user-scroll-up' });
    expect(state).toEqual(detached);
  });
});

// specs/0041 WS7.1 — append-time stick decision (replaces the old separate 100px check,
// unified onto the hysteresis thresholds).
describe('shouldStickToBottomOnAppend', () => {
  it('locked: sticks unconditionally, regardless of measured distance', () => {
    const locked: FollowState = { following: true, followLocked: true };
    expect(shouldStickToBottomOnAppend(locked, 0)).toBe(true);
    expect(shouldStickToBottomOnAppend(locked, 999)).toBe(true);
  });

  it('detached: never sticks', () => {
    const detached: FollowState = { following: false, followLocked: false };
    expect(shouldStickToBottomOnAppend(detached, 0)).toBe(false);
  });

  it('natural follow: sticks at the bottom, refuses after a real user scroll-up', () => {
    const following: FollowState = { following: true, followLocked: false };
    // Sitting at the (previous) bottom — content growth is excluded from the distance.
    expect(shouldStickToBottomOnAppend(following, 0)).toBe(true);
    // Within the hysteresis band: keep following.
    expect(shouldStickToBottomOnAppend(following, SCROLLED_UP_THRESHOLD_PX)).toBe(true);
    // The race guard: a scrollbar drag the debounced handler hasn't classified yet.
    expect(shouldStickToBottomOnAppend(following, SCROLLED_UP_THRESHOLD_PX + 1)).toBe(false);
  });
});

// specs/0029 WS4.1 — visibility of the "Jump to latest" pill in the transcript view.
describe('showJumpToLatest', () => {
  const base = {
    isRecording: true,
    following: false,
    disableAutoScroll: false,
    segmentCount: 12,
  };

  it('shows when detached during a live recording with content', () => {
    expect(showJumpToLatest(base)).toBe(true);
  });

  it('hides while following (nothing to jump to — already at the tail)', () => {
    expect(showJumpToLatest({ ...base, following: true })).toBe(false);
  });

  it('hides when not recording (post-meeting review has no live tail)', () => {
    expect(showJumpToLatest({ ...base, isRecording: false })).toBe(false);
  });

  it('hides when auto-scroll is disabled outright (meeting-details mode)', () => {
    expect(showJumpToLatest({ ...base, disableAutoScroll: true })).toBe(false);
  });

  it('hides when there are no segments', () => {
    expect(showJumpToLatest({ ...base, segmentCount: 0 })).toBe(false);
  });
});
