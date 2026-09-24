'use client';

import { useEffect } from 'react';
import { useRouter } from 'next/navigation';
import { CalendarX2 } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { CALENDAR_SETTINGS_ROUTE } from '@/components/Calendar/ConnectCalendarCard';
import { useGoogleCalendarConnect } from '@/hooks/useGoogleCalendarConnect';
import {
  GOOGLE_CALENDAR_AUTH_REQUIRED_EVENT,
  GOOGLE_CALENDAR_SYNCED_EVENT,
} from '@/lib/googleCalendar';
import { safeListen } from '@/lib/safe-listen';

/**
 * "Google Calendar needs reconnecting" — Today's compact row for a lapsed Google
 * grant (specs/0074 W2, 0075 W1).
 *
 * When Google's token refresh fails with `invalid_grant`, every later sync is a no-op
 * until the user reconnects. That used to be visible only if Settings → Calendar was
 * open at the moment it happened; Today kept showing the last-synced events as if they
 * were current. Today is the surface the owner actually looks at, so the latch shows
 * here, with the reconnect one click away.
 *
 * Self-contained: reads `authRequired` from the Google status on mount, on window focus,
 * and when the backend says the latch was set (`google-calendar-auth-required`) or a
 * pass landed (`google-calendar-synced`, which clears it after a reconnect elsewhere).
 * Renders nothing unless Google is connected AND the latch is set.
 */
export function GoogleReconnectRow() {
  const router = useRouter();
  const { status, refresh, connect, connecting } = useGoogleCalendarConnect();

  useEffect(() => {
    const onFocus = () => void refresh();
    window.addEventListener('focus', onFocus);
    const disposeAuth = safeListen(GOOGLE_CALENDAR_AUTH_REQUIRED_EVENT, () => void refresh());
    const disposeSynced = safeListen(GOOGLE_CALENDAR_SYNCED_EVENT, () => void refresh());
    return () => {
      window.removeEventListener('focus', onFocus);
      disposeAuth();
      disposeSynced();
    };
  }, [refresh]);

  if (!status?.connected || !status.authRequired) return null;

  return (
    <div
      role="status"
      className="mb-4 flex items-center gap-3 rounded-lg border border-border bg-card px-4 py-2.5"
    >
      <CalendarX2 className="h-4 w-4 flex-shrink-0 text-destructive" aria-hidden="true" />
      <div className="min-w-0 flex-1">
        <div className="text-sm font-medium text-foreground">
          Google Calendar needs reconnecting
        </div>
        <div className="truncate text-xs text-muted-foreground">
          Nothing has synced since your Google session expired — these may be out of date.
        </div>
      </div>
      <button
        type="button"
        onClick={() => router.push(CALENDAR_SETTINGS_ROUTE)}
        className="flex-shrink-0 text-xs text-muted-foreground hover:text-foreground"
      >
        Settings
      </button>
      <Button
        variant="brand"
        size="sm"
        className="flex-shrink-0"
        onClick={() => void connect()}
        disabled={connecting}
      >
        {connecting ? 'Finish in browser…' : 'Reconnect'}
      </Button>
    </div>
  );
}
