import { useCallback, useEffect, useRef, useState } from 'react';
import { toast } from 'sonner';
import {
  connectGoogleCalendar,
  getGoogleCalendarStatus,
  type GoogleCalendarStatus,
} from '@/lib/googleCalendar';

/** Result of a `connect()` call — mirrors `GoogleCalendarConnectResult`, flattened. */
export interface GoogleCalendarConnectOutcome {
  ok: boolean;
  email?: string;
  error?: string;
}

/**
 * Google Calendar connect flow, shared by the onboarding Calendar step and
 * `CalendarSettings` (specs/0061 W1 Task 3). Wraps `getGoogleCalendarStatus` /
 * `connectGoogleCalendar`, the connect-attempt sequence guard, and the two
 * connect toasts, so both call sites get identical behavior instead of a
 * second, drifting copy of `CalendarSettings`'s original `handleGoogleConnect`.
 *
 * The sequence guard (`connectSeq`) prevents a late-resolving connect call
 * (the backend waits up to 5 minutes for browser consent) from clobbering
 * `connecting` back to `false` out from under a NEWER connect attempt.
 * `cancel()` (specs/0061 W1 Task 3 fix, controller ruling R14) bumps the same
 * seq WITHOUT starting a new attempt, so a caller (e.g. a "Cancel"/"Dismiss"
 * button) can invalidate the in-flight call outright: its eventual
 * resolution becomes fully silent — no toast, no status refresh — instead of
 * surfacing minutes later as if nothing had been dismissed. `cancel()` also
 * clears `connecting` immediately so the caller's pending UI drops right away.
 */
export function useGoogleCalendarConnect() {
  const [connecting, setConnecting] = useState(false);
  const [status, setStatus] = useState<GoogleCalendarStatus | null>(null);
  const connectSeq = useRef(0);

  const refresh = useCallback(async () => {
    setStatus(await getGoogleCalendarStatus());
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const connect = useCallback(async (): Promise<GoogleCalendarConnectOutcome> => {
    const seq = ++connectSeq.current;
    setConnecting(true);
    const result = await connectGoogleCalendar();
    // Not current means either a newer connect() superseded this one, or
    // cancel() invalidated it — either way, nobody's waiting on THIS call
    // any more, so stay fully quiet: no toast, no status refresh.
    const isCurrent = connectSeq.current === seq;
    if (!isCurrent) {
      return result.ok ? { ok: true, email: result.email } : { ok: false, error: result.error };
    }

    setConnecting(false);

    if (result.ok) {
      toast.success('Google Calendar connected', {
        description: result.email || undefined,
      });
      await refresh();
      return { ok: true, email: result.email };
    }

    toast.error('Could not connect Google Calendar', { description: result.error });
    return { ok: false, error: result.error };
  }, [refresh]);

  /**
   * Invalidate the in-flight connect() call, if any, and immediately clear
   * `connecting`. The call keeps running underneath (its promise can't be
   * aborted), but its resolution — whenever it lands — now takes the
   * `!isCurrent` branch above: no toast, no refresh.
   */
  const cancel = useCallback(() => {
    connectSeq.current += 1;
    setConnecting(false);
  }, []);

  return { connecting, connect, status, refresh, cancel };
}
