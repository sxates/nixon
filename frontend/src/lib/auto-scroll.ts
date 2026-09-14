/**
 * Live-transcript auto-follow decision (specs/0019 WS1.2, specs/0041 WS7.1).
 *
 * While recording, the transcript should follow new segments to the bottom — but the
 * moment the user scrolls up to review earlier transcript, it must STAY put and not get
 * yanked back down on every new segment. The previous logic re-enabled following whenever
 * the user was within 100px of the bottom, so reviewing just above the end still jumped.
 *
 * This uses hysteresis with two thresholds:
 *  - re-attach only when the user is essentially AT the bottom (`AT_BOTTOM_THRESHOLD_PX`),
 *  - detach once they've scrolled up past `SCROLLED_UP_THRESHOLD_PX`,
 *  - in between, keep the current state (so a stray pixel doesn't flap the mode).
 *
 * specs/0041 WS7.1 layers explicit USER INTENT on top ("Jump to latest" kept detaching
 * because content growing under the viewport re-ran the hysteresis against an 8px
 * re-attach window): `nextFollowState` adds a sticky `followLocked` flag that only a
 * user-initiated upward scroll can clear — wheel/touch/keys, or a guarded upward
 * displacement the hook derives from scroll events (scrollTop decreased, scrollHeight
 * unchanged: a scrollbar-thumb drag). A raw `scroll` event never clears it, since it
 * also fires for programmatic scrolls and content growth.
 */

/** At/under this distance (px) from the bottom, resume live auto-follow. */
export const AT_BOTTOM_THRESHOLD_PX = 8;

/** Past this distance (px) from the bottom, treat the user as having scrolled up. */
export const SCROLLED_UP_THRESHOLD_PX = 24;

/**
 * Given the current distance from the bottom (px) and whether we're currently
 * following, return whether auto-follow should be on after this scroll.
 */
export function nextAutoFollow(
  distanceFromBottomPx: number,
  currentlyFollowing: boolean,
): boolean {
  if (distanceFromBottomPx <= AT_BOTTOM_THRESHOLD_PX) return true; // at bottom → follow
  if (distanceFromBottomPx > SCROLLED_UP_THRESHOLD_PX) return false; // scrolled up → detach
  return currentlyFollowing; // in-between → keep state (hysteresis)
}

/**
 * specs/0041 WS7.1 — the full follow state: hysteresis-driven natural follow, plus the
 * sticky user-intent lock set by "Jump to latest".
 */
export interface FollowState {
  /** Live auto-follow is on: new segments keep the view pinned to the tail. */
  following: boolean;
  /** Explicit user intent ("Jump to latest"). While set, following is unconditional —
   *  no proximity check can detach it; only a user-initiated upward scroll clears it. */
  followLocked: boolean;
}

export type FollowEvent =
  /** The user pressed "Jump to latest": follow, and LOCK the intent. */
  | { type: 'jump-to-latest' }
  /** User-initiated upward input (wheel up / touch drag / ArrowUp / PageUp / Home, or
   *  the hook's guarded scrollbar-drag displacement). The only thing that clears the
   *  lock; also detaches natural follow (hysteresis re-attaches the moment a scroll
   *  lands back at the bottom). */
  | { type: 'user-scroll-up' }
  /** A scroll event landed (programmatic, content growth, or scrollbar drag — the DOM
   *  can't tell us which, which is exactly why it can never clear the lock). */
  | { type: 'scroll'; distanceFromBottomPx: number };

/**
 * Pure follow-state reducer (specs/0041 WS7.1). `useAutoScroll` feeds it every relevant
 * event; the decision logic stays here so it's unit-testable without a DOM.
 */
export function nextFollowState(state: FollowState, event: FollowEvent): FollowState {
  switch (event.type) {
    case 'jump-to-latest':
      return { following: true, followLocked: true };
    case 'user-scroll-up':
      // Explicit intent to review: unlock and detach. If the user was merely nudging
      // at the bottom, the very next scroll event (≤ AT_BOTTOM_THRESHOLD_PX) re-attaches.
      return { following: false, followLocked: false };
    case 'scroll':
      // Locked: scroll events (programmatic scrolls, content growth) can NOT detach.
      if (state.followLocked) return state;
      return {
        ...state,
        following: nextAutoFollow(event.distanceFromBottomPx, state.following),
      };
  }
}

/**
 * Append-time decision (specs/0041 WS7.1): when new segments arrive, should the view
 * stick to the bottom? Collapses the old duplicated 100px imperative check into the one
 * hysteresis helper. `distanceFromPreviousBottomPx` must be measured against the scroll
 * height as it was BEFORE this append (the new rows grow scrollHeight, and content
 * growth must never read as the user having scrolled up) — it is only a race guard for
 * scrollbar drags the debounced scroll handler hasn't classified yet.
 */
export function shouldStickToBottomOnAppend(
  state: FollowState,
  distanceFromPreviousBottomPx: number,
): boolean {
  if (state.followLocked) return true; // user intent: unconditional
  if (!state.following) return false;
  return nextAutoFollow(distanceFromPreviousBottomPx, true);
}

/**
 * Whether the "Jump to latest" affordance should be visible (specs/0029 WS4.1).
 *
 * Shown only while a live recording is appending segments AND the user has detached
 * (scrolled up, so auto-follow is off). Never shown when auto-scroll is disabled
 * outright (meeting-details review mode) or when there is nothing to jump to.
 */
export function showJumpToLatest(args: {
  isRecording: boolean;
  following: boolean;
  disableAutoScroll: boolean;
  segmentCount: number;
}): boolean {
  const { isRecording, following, disableAutoScroll, segmentCount } = args;
  if (disableAutoScroll) return false;
  if (!isRecording) return false;
  if (segmentCount === 0) return false;
  return !following;
}
