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
 * `connecting` back to `false` out from under a newer connect attempt, and
 * from toasting over whatever the newer attempt is doing. A successful
 * connect still refreshes `status` and can still toast even if it's no
 * longer the "current" attempt — only the loading flag and toast are gated.
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
    const isCurrent = connectSeq.current === seq;
    if (isCurrent) setConnecting(false);

    if (result.ok) {
      if (isCurrent) {
        toast.success('Google Calendar connected', {
          description: result.email || undefined,
        });
      }
      await refresh();
      return { ok: true, email: result.email };
    }

    if (isCurrent) {
      toast.error('Could not connect Google Calendar', { description: result.error });
    }
    return { ok: false, error: result.error };
  }, [refresh]);

  return { connecting, connect, status, refresh };
}
