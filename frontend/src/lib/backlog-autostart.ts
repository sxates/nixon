/**
 * Deferred-backlog auto-start decision (spec 0045 WS2).
 *
 * When a backlog of unprocessed (low-power/deferred) meetings exists AND the machine is on
 * AC power, processing auto-starts — no prompt. On battery it never auto-starts (that would
 * defeat the battery-saving purpose); the user can still start it manually. This is the pure,
 * unit-testable decision behind the trigger in useDeferredBacklog.
 */

/** Short debounce after a triggering signal (power change / mount / enqueue) before we act,
 *  to absorb brief power blips. Replaces the old 90s/120s prompt debounces. */
export const AUTOSTART_DEBOUNCE_MS = 5000;

export function decideAutostart(input: {
  backlogCount: number;
  onBattery: boolean;
  isProcessing: boolean;
}): boolean {
  return input.backlogCount > 0 && !input.onBattery && !input.isProcessing;
}
