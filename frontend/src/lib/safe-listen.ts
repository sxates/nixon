/**
 * Safe Tauri event listener helpers.
 *
 * Tauri's `_unlisten(event, eventId)` does `listeners[eventId].handlerId` and
 * THROWS if `listeners[eventId]` is undefined — i.e. if `unlisten()` is called
 * for a listener Tauri has already removed. In React 18 (and StrictMode), effect
 * cleanups can run more than once, and `listen()` is async, so the classic
 * pattern
 *
 *     useEffect(() => {
 *       let un;
 *       listen(...).then(f => un = f);
 *       return () => un?.();
 *     }, [deps]);
 *
 * is unsafe in two ways:
 *   1. Async-listen race: if the effect unmounts/re-runs before the promise
 *      resolves, the cleanup runs while `un` is still undefined, so the listener
 *      leaks AND is later registered with no owner to remove it.
 *   2. Double-unlisten: if the returned cleanup is invoked more than once (React
 *      commit edge cases, or two code paths both calling it), Tauri throws
 *      `undefined is not an object (evaluating 'listeners[eventId].handlerId')`
 *      DURING the React commit phase, which crashes the app.
 *
 * `safeListen` returns a cleanup function immediately (no promise to await in the
 * effect body) that is:
 *   - race-safe: if cleanup runs before `listen()` resolves, we mark the
 *     subscription disposed and unlisten the moment the real handle arrives.
 *   - idempotent: calling the returned cleanup more than once is a no-op.
 *   - crash-proof: the underlying `unlisten()` is wrapped in try/catch so a Tauri
 *     "already removed" throw can never crash React's commit phase.
 *
 * Usage:
 *     useEffect(() => safeListen('my-event', handler), []);
 *  or, when you need the value inside an async setup:
 *     useEffect(() => {
 *       const dispose = safeListen('my-event', handler);
 *       return dispose;
 *     }, []);
 */

import { listen, type EventName, type EventCallback, type UnlistenFn } from '@tauri-apps/api/event';

/**
 * Wrap a single resolved/awaited Tauri unlisten function so that:
 *   - it only ever runs once (subsequent calls are no-ops), and
 *   - a throw from Tauri (already-removed listener) is swallowed + warned.
 *
 * Use this when you already have an `UnlistenFn` from an `await listen(...)`.
 */
export function makeSafeUnlisten(unlisten: UnlistenFn | undefined): () => void {
  let called = false;
  return () => {
    if (called) return;
    called = true;
    try {
      // Tauri v2's UnlistenFn is async (it calls `invoke('plugin:event|unlisten')`), so when the
      // listener is already removed it rejects a PROMISE rather than throwing synchronously — a
      // plain try/catch would miss it and it'd surface as an unhandled rejection (crashing the
      // Next dev overlay mid-commit). Swallow both the sync throw and the async rejection.
      const result = unlisten?.() as unknown;
      if (result && typeof (result as Promise<unknown>).then === 'function') {
        (result as Promise<unknown>).catch((err) => {
          console.warn('[safe-listen] async unlisten rejected (listener likely already removed):', err);
        });
      }
    } catch (err) {
      console.warn('[safe-listen] unlisten threw (listener likely already removed):', err);
    }
  };
}

/**
 * Register a Tauri event listener and return a synchronous, idempotent,
 * crash-proof cleanup function suitable for returning directly from a React
 * effect.
 *
 * The returned cleanup handles the async-listen race: if it is invoked before
 * `listen()` resolves, the listener is torn down as soon as the handle arrives.
 */
export function safeListen<T>(
  event: EventName,
  handler: EventCallback<T>,
): () => void {
  let disposed = false;
  let safeUnlisten: (() => void) | undefined;

  listen<T>(event, handler)
    .then((unlisten) => {
      safeUnlisten = makeSafeUnlisten(unlisten);
      // Cleanup already ran before the listener resolved — tear it down now.
      if (disposed) {
        safeUnlisten();
      }
    })
    .catch((err) => {
      console.error(`[safe-listen] failed to register listener for "${String(event)}":`, err);
    });

  return () => {
    disposed = true;
    // If the listener has resolved, this unlistens (once, crash-proof).
    // If not, the `.then` above will unlisten on resolve because `disposed` is set.
    safeUnlisten?.();
  };
}
