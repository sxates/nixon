'use client';

import { useCallback, useEffect, useState } from 'react';
import { listen } from '@tauri-apps/api/event';
import { toast } from 'sonner';
import { Loader2, RefreshCw } from 'lucide-react';
import { Button } from '@/components/ui/button';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog';
import {
  type CalendarAccessStatus,
  getCalendarAccessStatus,
  requestCalendarAccess,
} from '@/lib/calendar';
import {
  GOOGLE_CALENDAR_AUTH_REQUIRED_EVENT,
  type GoogleCalendarStatus,
  type GoogleCapabilities,
  capabilitiesPending,
  capabilityState,
  disconnectGoogleCalendar,
  formatLastSynced,
  getGoogleCapabilities,
  setGoogleCalendarSelected,
  setGoogleCalendarsSelected,
  syncGoogleCalendarNow,
} from '@/lib/googleCalendar';
import { useGoogleCalendarConnect } from '@/hooks/useGoogleCalendarConnect';
import { SettingsNote, SettingsSection } from '@/components/ui/settings';

/** Human-readable label for each EventKit access status. */
const CALENDAR_STATUS_LABEL: Record<CalendarAccessStatus, string> = {
  authorized: 'Connected',
  denied: 'Access denied',
  notDetermined: 'Not connected',
  restricted: 'Restricted by system',
};

/**
 * ADR-0010 privacy copy — verbatim; do not reword without amending the ADR.
 * Kept as a single string so it renders as one text node (and tests can match
 * it exactly).
 */
const GOOGLE_PRIVACY_COPY =
  'Read-only. Nixon downloads event details (titles, times, attendees) to this Mac. ' +
  'Your recordings, transcripts, and notes are never uploaded — to Google or anyone.';

/**
 * Supplementary disclosure for the best-effort enrichment scopes (specs/0038 WS3,
 * ADR-0010 amendment). Kept SEPARATE from the verbatim ADR-0010 privacy copy above so
 * that copy stays untouched. Honest about the extra reads and the graceful-degrade floor.
 */
const GOOGLE_ENRICHMENT_COPY =
  'When your organization allows it, Nixon also reads group membership (to list who a ' +
  'distribution list invites) and directory profile photos — read-only, and stored only ' +
  'on this Mac. If your organization restricts them, meetings simply show the group ' +
  'address and initials, with nothing extra downloaded.';

/** One "Enhanced attendee details" status line, driven by a tri-state capability flag. */
function CapabilityRow({
  label,
  flag,
  onCopy,
  offCopy,
}: {
  label: string;
  flag: boolean | null;
  /** Text when the org allows it (flag true). */
  onCopy: string;
  /** Text when consented but denied by org policy (flag false). */
  offCopy: string;
}) {
  const state = capabilityState(flag);
  const value =
    state === 'checking' ? 'checking…' : state === 'on' ? onCopy : offCopy;
  const valueClass =
    state === 'on'
      ? 'text-foreground'
      : state === 'off'
        ? 'text-muted-foreground'
        : 'text-muted-foreground italic';
  return (
    <div className="flex items-center justify-between gap-3 text-sm">
      <span className="text-muted-foreground">{label}</span>
      <span className={valueClass}>{value}</span>
    </div>
  );
}

/**
 * Badge marking the row that is Nixon's active calendar source — same house
 * style as SOURCE_BADGE_CLASS in TemplateSettings (the "custom" variant).
 */
const ACTIVE_BADGE_CLASS =
  'flex-shrink-0 rounded-[3px] px-2 py-0.5 text-[11px] font-semibold bg-brand/10 text-brand';

/**
 * Settings → Calendar card (specs/0032 task 6).
 *
 * Nixon uses a SINGLE active calendar source — no merged view. While Google
 * Calendar is connected it is the only source; otherwise macOS Calendar is.
 * `connected: true` from `api_google_calendar_status` means "Google is the
 * active source". The card presents the two rows as a source choice:
 *  - macOS Calendar: the pre-existing EventKit permission UI (behavior
 *    unchanged); carries the "Active" badge while Google is disconnected, and
 *    is de-emphasized ("Not used while Google Calendar is connected") while
 *    Google is the source.
 *  - Google Calendar: the opt-in read-only provider (specs/0032, ADR-0010) with
 *    four states — not configured in this build / disconnected (Connect + privacy
 *    copy) / connecting (browser consent pending) / connected (email, last sync,
 *    per-calendar toggles, Sync now, Disconnect-with-confirm). Connecting =
 *    choosing Google; disconnecting = back to macOS Calendar.
 *
 * Also listens for the Rust `google-calendar-auth-required` event (stored token
 * revoked/expired) → sonner toast + a persistent reconnect banner in this card.
 */
export function CalendarSettings() {
  // --- macOS Calendar (EventKit) — moved from RecordingSettings, unchanged. ---
  const [calendarStatus, setCalendarStatus] = useState<CalendarAccessStatus | null>(null);
  const [connectingCalendar, setConnectingCalendar] = useState(false);

  // --- Google Calendar (specs/0032) — status/connect/sequence-guard/toasts
  // now live in the shared `useGoogleCalendarConnect` hook (specs/0061 W1
  // Task 3), which the onboarding Calendar step also uses.
  const {
    connecting: googleConnecting,
    connect: hookConnect,
    status: hookGoogleStatus,
    refresh: refreshGoogleStatus,
    cancel: cancelGoogleConnect,
  } = useGoogleCalendarConnect();
  /**
   * Local shadow of the hook's status, kept in sync via the effect below.
   * Needed (rather than using `hookGoogleStatus` directly) so the per-calendar
   * toggle / bulk-toggle handlers can apply an OPTIMISTIC update and revert it
   * on failure, same as before the hook extraction — the hook itself only
   * owns connect/refresh, not arbitrary local mutations.
   */
  const [googleStatus, setGoogleStatus] = useState<GoogleCalendarStatus | null>(null);
  useEffect(() => {
    setGoogleStatus(hookGoogleStatus);
  }, [hookGoogleStatus]);
  const [googleSyncing, setGoogleSyncing] = useState(false);
  const [authRequired, setAuthRequired] = useState(false);
  const [disconnectOpen, setDisconnectOpen] = useState(false);
  const [disconnecting, setDisconnecting] = useState(false);
  // Probed best-effort enrichment capabilities (specs/0038 WS3); null until fetched.
  const [capabilities, setCapabilities] = useState<GoogleCapabilities | null>(null);

  // Load EventKit status on mount (spec 0008 behavior); Google status load is
  // handled inside useGoogleCalendarConnect.
  useEffect(() => {
    void getCalendarAccessStatus().then(setCalendarStatus);
  }, []);

  // Reconnect prompt (specs/0032): backend emits this when a token refresh fails
  // with invalid_grant (access revoked / expired). Toast + persistent banner.
  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | undefined;
    listen(GOOGLE_CALENDAR_AUTH_REQUIRED_EVENT, () => {
      setAuthRequired(true);
      toast.error('Google Calendar disconnected', {
        description:
          'Your Google session expired or was revoked. Reconnect in Settings → Calendar.',
      });
      void refreshGoogleStatus();
    })
      .then((fn) => {
        if (cancelled) fn();
        else unlisten = fn;
      })
      .catch((err) => {
        console.warn('[CalendarSettings] auth-required listener failed:', err);
      });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [refreshGoogleStatus]);

  // Fetch + poll the best-effort enrichment capabilities while Google is connected
  // (specs/0038 WS3). The connect-time probe is fire-and-forget, so a just-connected
  // account reports `null` flags ("checking…"); re-fetch a few times until they
  // populate, then stop. Cleared to null when disconnected.
  const googleConnected = googleStatus?.connected === true;
  useEffect(() => {
    if (!googleConnected) {
      setCapabilities(null);
      return;
    }
    let cancelled = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    let attempts = 0;
    const poll = async () => {
      const caps = await getGoogleCapabilities();
      if (cancelled) return;
      setCapabilities(caps);
      attempts += 1;
      // The probe usually lands within a few seconds; bound the retries either way.
      if (capabilitiesPending(caps) && attempts < 6) {
        timer = setTimeout(() => void poll(), 2500);
      }
    };
    void poll();
    return () => {
      cancelled = true;
      if (timer) clearTimeout(timer);
    };
  }, [googleConnected]);

  // EventKit connect — unchanged from RecordingSettings.
  const handleConnectCalendar = useCallback(async () => {
    setConnectingCalendar(true);
    try {
      const granted = await requestCalendarAccess();
      if (granted) {
        toast.success('Calendar connected');
      } else {
        toast.error('Calendar access not granted', {
          description:
            'You can grant access in System Settings → Privacy & Security → Calendars.',
        });
      }
    } finally {
      setConnectingCalendar(false);
      setCalendarStatus(await getCalendarAccessStatus());
    }
  }, []);

  // Google connect / reconnect: long-running (browser consent, ≤5 min).
  // Status/sequence-guard/toasts live in useGoogleCalendarConnect.
  const handleGoogleConnect = useCallback(async () => {
    const result = await hookConnect();
    if (result.ok) {
      setAuthRequired(false);
    }
  }, [hookConnect]);

  // Dismiss the pending state. The backend command itself times out after 5
  // minutes, so the underlying invoke keeps running — but `cancel()`
  // invalidates it in the hook, so its eventual resolution is fully silent
  // (no toast, no status refresh) instead of surfacing minutes after the
  // user gave up (specs/0061 W1 Task 3 fix, controller ruling R14).
  const handleGoogleConnectDismiss = useCallback(() => {
    cancelGoogleConnect();
  }, [cancelGoogleConnect]);

  // Per-calendar sync toggle — optimistic, revert on failure (house pattern).
  const handleCalendarToggle = useCallback(
    async (calendarId: string, selected: boolean) => {
      const previous = googleStatus;
      if (!previous) return;
      setGoogleStatus({
        ...previous,
        calendars: previous.calendars.map((c) =>
          c.id === calendarId ? { ...c, selected } : c,
        ),
      });
      const ok = await setGoogleCalendarSelected(calendarId, selected);
      if (!ok) {
        setGoogleStatus(previous); // revert
        toast.error('Could not update calendar selection');
      }
    },
    [googleStatus],
  );

  // Bulk Select all / none (specs/0041 WS5) — optimistic like the single toggle,
  // but through the batched backend command (one sync pass instead of N).
  const [bulkToggling, setBulkToggling] = useState(false);
  const handleBulkToggle = useCallback(
    async (selected: boolean) => {
      const previous = googleStatus;
      if (!previous || bulkToggling) return;
      // Only calendars actually changing state go to the backend.
      const changing = previous.calendars
        .filter((c) => c.selected !== selected)
        .map((c) => c.id);
      if (changing.length === 0) return;
      setBulkToggling(true);
      setGoogleStatus({
        ...previous,
        calendars: previous.calendars.map((c) => ({ ...c, selected })),
      });
      const ok = await setGoogleCalendarsSelected(changing, selected);
      setBulkToggling(false);
      if (!ok) {
        setGoogleStatus(previous); // revert
        toast.error('Could not update calendar selection');
      }
    },
    [googleStatus, bulkToggling],
  );

  const handleSyncNow = useCallback(async () => {
    setGoogleSyncing(true);
    const ok = await syncGoogleCalendarNow();
    setGoogleSyncing(false);
    if (ok) {
      toast.success('Google Calendar synced');
      await refreshGoogleStatus();
    } else {
      toast.error('Sync failed', {
        description: 'Nixon will retry automatically. Check your connection and try again.',
      });
    }
  }, [refreshGoogleStatus]);

  const handleDisconnect = useCallback(async () => {
    if (disconnecting) return;
    setDisconnecting(true);
    const ok = await disconnectGoogleCalendar();
    setDisconnecting(false);
    if (ok) {
      setDisconnectOpen(false);
      setAuthRequired(false);
      toast.success('Google Calendar disconnected', {
        description:
          "Nixon is back to using your Mac's calendar. The local copy of your Google events was deleted.",
      });
      await refreshGoogleStatus();
    } else {
      toast.error('Could not disconnect Google Calendar', {
        description: 'Please try again.',
      });
    }
  }, [disconnecting, refreshGoogleStatus]);

  const connected = googleStatus?.connected === true;
  const configured = googleStatus?.configured === true;

  return (
    <SettingsSection
      title="Calendar"
      description="Nixon uses one calendar source — your Mac's calendar, or Google Calendar connected directly."
    >
      {/* Reconnect banner — persistent until reconnect/disconnect (specs/0032). */}
      {authRequired && configured && (
        <SettingsNote role="status" tone="info" className="flex items-center justify-between gap-4">
          <div className="flex-1">
            <span className="font-medium">Google Calendar disconnected — reconnect</span>
            <div className="mt-0.5">
              Your Google session expired or was revoked. Your meetings still show from
              macOS Calendar and the last-synced Google events.
            </div>
          </div>
          <Button
            variant="brand"
            size="sm"
            className="flex-shrink-0"
            onClick={handleGoogleConnect}
            disabled={googleConnecting}
          >
            {googleConnecting ? 'Reconnecting…' : 'Reconnect'}
          </Button>
        </SettingsNote>
      )}

      {/* macOS Calendar (EventKit) — permission behavior unchanged, relocated
          from RecordingSettings. Read on-device; zero network egress. Active
          source whenever Google is not connected; de-emphasized (but still
          usable) while Google is the source. */}
      <div
        role="group"
        aria-label="macOS Calendar source"
        className={`flex items-center justify-between gap-4 rounded-[3px] border border-border bg-card p-4 ${
          connected ? 'opacity-60' : ''
        }`}
      >
        <div className="flex-1">
          <div className="flex items-center gap-2 font-medium">
            macOS Calendar
            {!connected && <span className={ACTIVE_BADGE_CLASS}>Active</span>}
          </div>
          <div className="text-sm text-muted-foreground">
            {connected ? (
              <>
                Not used while Google Calendar is connected
                {' · '}
                {calendarStatus ? CALENDAR_STATUS_LABEL[calendarStatus] : 'Checking…'}
              </>
            ) : (
              <>
                {calendarStatus ? CALENDAR_STATUS_LABEL[calendarStatus] : 'Checking…'}
                {' — '}read on-device; your calendar data never leaves this Mac.
              </>
            )}
          </div>
        </div>
        {calendarStatus !== 'authorized' && (
          <Button
            variant={connected ? 'outline' : 'brand'}
            size="sm"
            onClick={handleConnectCalendar}
            disabled={connectingCalendar || calendarStatus === 'restricted'}
          >
            {connectingCalendar ? 'Connecting…' : 'Connect Calendar'}
          </Button>
        )}
      </div>

      {/* Google Calendar (specs/0032, ADR-0010) — connecting makes it the
          active (and only) calendar source. */}
      <div role="group" aria-label="Google Calendar source" className="rounded-[3px] border border-border bg-card p-4">
        <div className="flex items-center justify-between gap-4">
          <div className="min-w-0 flex-1">
            <div className="flex items-center gap-2 font-medium">
              Google Calendar
              {connected && <span className={ACTIVE_BADGE_CLASS}>Active</span>}
            </div>
            <div className="text-sm text-muted-foreground">
              {googleStatus === null
                ? 'Checking…'
                : !configured
                  ? // State (a): no OAuth client id baked into this build — inert.
                    "Google Calendar isn't configured in this build."
                  : googleConnecting
                    ? 'Complete the connection in your browser…'
                    : connected
                      ? `Connected as ${googleStatus.email ?? 'your Google account'} · ${formatLastSynced(googleStatus.lastSyncedAt)}`
                      : 'Not connected'}
            </div>
          </div>

          {/* State (c): connect in flight — cancel/dismiss affordance. */}
          {configured && googleConnecting && (
            <div className="flex flex-shrink-0 items-center gap-2">
              <Loader2 className="h-4 w-4 animate-spin text-muted-foreground" aria-hidden />
              <Button variant="outline" size="sm" onClick={handleGoogleConnectDismiss}>
                Cancel
              </Button>
            </div>
          )}

          {/* State (b): disconnected — Connect. */}
          {configured && !googleConnecting && !connected && (
            <Button
              variant="brand"
              size="sm"
              className="flex-shrink-0"
              onClick={handleGoogleConnect}
            >
              Connect Google Calendar
            </Button>
          )}

          {/* State (d): connected — Sync now + Disconnect. */}
          {configured && !googleConnecting && connected && (
            <div className="flex flex-shrink-0 items-center gap-2">
              <Button
                variant="outline"
                size="sm"
                onClick={handleSyncNow}
                disabled={googleSyncing}
              >
                <RefreshCw className={`h-4 w-4 ${googleSyncing ? 'animate-spin' : ''}`} />
                {googleSyncing ? 'Syncing…' : 'Sync now'}
              </Button>
              <Button
                variant="outline"
                size="sm"
                className="text-destructive hover:text-destructive"
                onClick={() => setDisconnectOpen(true)}
              >
                Disconnect
              </Button>
            </div>
          )}
        </div>

        {/* State (b) details: source-switch note + privacy copy (ADR-0010,
            verbatim) + first-connect note. */}
        {configured && !connected && !googleConnecting && (
          <div className="mt-3 space-y-2 border-t pt-3">
            <p className="text-sm text-muted-foreground">
              Connecting switches Nixon&apos;s calendar source to Google Calendar.
            </p>
            <p className="text-sm text-muted-foreground">{GOOGLE_PRIVACY_COPY}</p>
            <p className="text-sm text-muted-foreground">{GOOGLE_ENRICHMENT_COPY}</p>
            <p className="text-xs text-muted-foreground">
              First time connecting: Google shows an &quot;unverified app&quot; warning —
              expected for a personal build. Click Advanced → Continue.
            </p>
          </div>
        )}

        {/* State (c) details: waiting on browser consent. */}
        {configured && googleConnecting && (
          <div className="mt-3 border-t pt-3">
            <p className="text-sm text-muted-foreground">
              Finish signing in with Google in the browser window that just opened. This
              times out after 5 minutes.
            </p>
          </div>
        )}

        {/* State (d) details: per-calendar sync toggles. */}
        {configured && connected && !googleConnecting && googleStatus && (
          <div className="mt-3 border-t pt-3">
            <div className="mb-2 flex items-center justify-between gap-3">
              <div className="text-sm font-medium">Calendars to sync</div>
              {googleStatus.calendars.length > 1 && (
                <div className="flex items-center gap-1 text-xs">
                  <button
                    type="button"
                    onClick={() => void handleBulkToggle(true)}
                    disabled={bulkToggling}
                    className="rounded px-1.5 py-0.5 font-medium text-brand hover:underline disabled:opacity-50 focus:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                  >
                    Select all
                  </button>
                  <span className="text-border" aria-hidden="true">
                    ·
                  </span>
                  <button
                    type="button"
                    onClick={() => void handleBulkToggle(false)}
                    disabled={bulkToggling}
                    className="rounded px-1.5 py-0.5 font-medium text-brand hover:underline disabled:opacity-50 focus:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                  >
                    Select none
                  </button>
                </div>
              )}
            </div>
            {googleStatus.calendars.length > 0 ? (
              <ul className="space-y-1.5">
                {googleStatus.calendars.map((calendar) => (
                  <li key={calendar.id}>
                    <label className="flex cursor-pointer items-center gap-2 text-sm text-foreground">
                      <input
                        type="checkbox"
                        checked={calendar.selected}
                        onChange={(e) =>
                          void handleCalendarToggle(calendar.id, e.target.checked)
                        }
                        className="h-4 w-4 accent-brand"
                      />
                      <span className="truncate">{calendar.summary}</span>
                    </label>
                  </li>
                ))}
              </ul>
            ) : (
              <p className="text-sm text-muted-foreground">
                No calendars found on this account yet. Try &quot;Sync now&quot;.
              </p>
            )}
            {/* Enhanced attendee details (specs/0038 WS3): honest per-account status
                for the two best-effort scopes. `checking…` while the fire-and-forget
                probe is still running (the effect re-fetches until it lands). */}
            <div className="mt-4 border-t pt-3">
              <div className="mb-1.5 text-sm font-medium">Enhanced attendee details</div>
              <div className="space-y-1">
                <CapabilityRow
                  label="Distribution lists"
                  flag={capabilities?.canExpandGroups ?? null}
                  onCopy="expanding to members"
                  offCopy="not available on this account"
                />
                <CapabilityRow
                  label="Attendee photos"
                  flag={capabilities?.canFetchPhotos ?? null}
                  onCopy="on"
                  offCopy="not available"
                />
              </div>
              {/* RC-2: break the silence when the org restricts group expansion —
                  an explicit "why" so the `off` state isn't a bare label. */}
              {capabilityState(capabilities?.canExpandGroups ?? null) === 'off' && (
                <p className="mt-2 text-xs text-muted-foreground">
                  Your organization restricts Google group access, so distribution lists
                  can&apos;t be expanded to their members. Everything else (attendee names,
                  photos) still works.
                </p>
              )}
              <p className="mt-2 text-xs text-muted-foreground">
                Availability depends on your organization&apos;s directory and group-visibility
                policy — a granted permission isn&apos;t access if your admin restricts it.
              </p>
            </div>
            <p className="mt-3 text-xs text-muted-foreground">{GOOGLE_PRIVACY_COPY}</p>
            <p className="mt-2 text-xs text-muted-foreground">{GOOGLE_ENRICHMENT_COPY}</p>
          </div>
        )}
      </div>

      {/* Disconnect confirm (house dialog pattern — DeleteTemplateDialog). */}
      <Dialog
        open={disconnectOpen}
        onOpenChange={(next) => (disconnecting ? undefined : setDisconnectOpen(next))}
      >
        <DialogContent className="max-w-md">
          <DialogHeader>
            <DialogTitle>Disconnect Google Calendar?</DialogTitle>
            <DialogDescription>
              Nixon will go back to using your Mac&apos;s calendar. The local copy of
              your Google events is deleted. You can reconnect anytime.
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button
              variant="outline"
              onClick={() => setDisconnectOpen(false)}
              disabled={disconnecting}
            >
              Cancel
            </Button>
            <Button variant="destructive" onClick={handleDisconnect} disabled={disconnecting}>
              {disconnecting && <Loader2 className="mr-2 h-4 w-4 animate-spin" />}
              Disconnect Google Calendar
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </SettingsSection>
  );
}
