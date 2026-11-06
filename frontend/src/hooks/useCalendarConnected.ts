import { useEffect, useState } from 'react';
import { isAnyCalendarConnected } from '@/lib/calendar';
import { safeListen } from '@/lib/safe-listen';

/**
 * Whether ANY calendar source is connected (EventKit or Google, specs/0069 W4) — the same
 * `isAnyCalendarConnected` the connect-calendar nudge has always been gated on. `null`
 * until the first check resolves.
 *
 * Re-checked when the window regains focus (the usual way back from System Settings or
 * Settings → Calendar) and after a Google sync pass, so a calendar that disconnects
 * mid-session moves the Meetings page to its no-calendar view instead of leaving it on an
 * empty agenda.
 */
export function useCalendarConnected(): boolean | null {
  const [connected, setConnected] = useState<boolean | null>(null);

  useEffect(() => {
    let cancelled = false;
    const check = () => {
      void isAnyCalendarConnected().then((c) => {
        if (!cancelled) setConnected(c);
      });
    };
    check();
    window.addEventListener('focus', check);
    const disposeGoogle = safeListen('google-calendar-synced', check);
    return () => {
      cancelled = true;
      window.removeEventListener('focus', check);
      disposeGoogle();
    };
  }, []);

  return connected;
}
