'use client';

import { useEffect, useState } from 'react';
import { ConnectCalendarCard } from '@/components/Calendar/ConnectCalendarCard';

/**
 * Compact "connect your calendar" nudge (mirrors the Day Agenda affordance).
 *
 * specs/0066: this used to ask EventKit for access itself and, when the answer was no,
 * toast "grant access in System Settings" — sending you off to find a setting rather than
 * taking you to one. The card routes to Settings → Calendar now, where both sources are
 * offered and the permission prompt belongs, so there is nothing left here to handle.
 *
 * specs/0069 W4 (Ruling 2): the caller (Home) already gates rendering on
 * `calendarConnected === false` — a real connection makes the nudge disappear on its
 * own, no dismiss needed. This local dismiss is for the OTHER case: "I know, I'll get to
 * it" while still disconnected. Persisted so it doesn't reappear every reload; wrapped in
 * try/catch since `localStorage` throws in private windows (Today must still render).
 */
const DISMISSED_KEY = 'nixon.today.calendarNudgeDismissed';

function readDismissed(): boolean {
  try {
    return localStorage.getItem(DISMISSED_KEY) === '1';
  } catch {
    return false;
  }
}

export function ConnectCalendarNudge() {
  // Starts visible and hides itself once the persisted dismissal is read in the effect
  // below — a one-frame flash for a returning dismisser, not fixed with a lazy `useState`
  // initializer. This page is a static export: the prerendered HTML has no `localStorage`,
  // so hydration requires the FIRST client render to match that HTML exactly; reading the
  // dismissal in the initializer would mismatch (and `useSyncExternalStore` settles on the
  // same post-mount timing, so it wouldn't remove the flash either — just formalize this
  // shape). Confirmed with review (specs/0069 W4 fix round 1): this trade-off stands.
  const [dismissed, setDismissed] = useState(false);

  useEffect(() => {
    setDismissed(readDismissed());
  }, []);

  if (dismissed) return null;

  return (
    <ConnectCalendarCard
      title="Connect your calendar to see your full day"
      onDismiss={() => {
        try {
          localStorage.setItem(DISMISSED_KEY, '1');
        } catch {
          /* private window — the state update below still hides it for this session */
        }
        setDismissed(true);
      }}
    />
  );
}
