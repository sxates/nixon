/**
 * In-app navigation history, so a Back button can tell whether the previous history entry
 * is a page of this session (go back to it) or not (go to the page's fallback instead).
 *
 * `window.history.length` cannot answer that: the Tauri webview starts with entries of its
 * own, so a cold deep link reports a history that `router.back()` would leave the app for.
 *
 * Every `pushState` / `replaceState` (the Next router's own calls included) is stamped with
 * this entry's position in the session, `__nixonNavIdx`; a `popstate` reads the stamp back.
 * Position 0 is the page the app opened on, so there is somewhere to go back to exactly
 * when the current position is above 0. Next keeps its own keys in `history.state`; ours
 * rides beside them.
 */

const KEY = '__nixonNavIdx';

let index = 0;
let installed = false;

type StateFn = History['pushState'];

const stampOf = (state: unknown): number | null => {
  if (state && typeof state === 'object' && KEY in state) {
    const n = (state as Record<string, unknown>)[KEY];
    return typeof n === 'number' && Number.isFinite(n) ? n : null;
  }
  return null;
};

const stamp = (state: unknown, idx: number) =>
  state && typeof state === 'object' ? { ...(state as object), [KEY]: idx } : { [KEY]: idx };

/** Starts tracking. Idempotent; a no-op outside the browser. */
export function installNavHistory(): void {
  if (installed || typeof window === 'undefined') return;
  installed = true;
  const h = window.history;
  // A reload keeps the history entries, and with them our stamps: resume from there.
  index = stampOf(h.state) ?? 0;

  const push: StateFn = h.pushState.bind(h);
  const replace: StateFn = h.replaceState.bind(h);
  h.pushState = (state, unused, url) => {
    index += 1;
    push(stamp(state, index), unused, url);
  };
  h.replaceState = (state, unused, url) => {
    replace(stamp(state, index), unused, url);
  };
  window.addEventListener('popstate', (e) => {
    // An entry from before this session carries no stamp: nothing in-app is behind it.
    index = stampOf(e.state) ?? 0;
  });
}

/** True when the entry behind the current one is a page of this session. */
export function canGoBackInApp(): boolean {
  return index > 0;
}

/** Test hook: forget the session (does not unpatch `history`). */
export function resetNavHistoryForTests(idx = 0): void {
  index = idx;
}
