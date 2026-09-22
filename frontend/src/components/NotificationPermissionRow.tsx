'use client';

/**
 * Settings → General → Notifications: the live macOS permission (specs/0068).
 *
 * This row replaces a switch labelled "Meeting start and end notifications", which wrote a
 * setting nothing read — and sat in a section where, until now, nothing could be delivered
 * at all. What a person needs here is the state macOS is actually in, a way into it, and a
 * way to prove a banner arrives.
 *
 * The three states are different questions, so they get different controls:
 *   - not yet asked → **Allow…**, which shows the macOS dialog.
 *   - refused → **Open System Settings**, because macOS will not ask twice; the stored
 *     answer comes back without a dialog and a second "Allow…" would look broken.
 *   - allowed → **Send a test notification**, the only way to see it working without
 *     waiting for a meeting.
 */

import { useCallback, useEffect, useState } from 'react';
import { Bell, Check, Loader2 } from 'lucide-react';
import { toast } from 'sonner';
import {
  type AuthorizationStatus,
  type NotificationCapability,
  CATEGORY_PLAIN,
  getNotificationCapability,
  getNotificationPermission,
  notify,
  openNotificationSettings,
  requestNotificationPermission,
} from '@/lib/osNotification';
import { Button } from '@/components/ui/button';
import { SettingsNote, SettingsRow } from '@/components/ui/settings';

export function NotificationPermissionRow() {
  const [capability, setCapability] = useState<NotificationCapability | null>(null);
  const [status, setStatus] = useState<AuthorizationStatus | null>(null);
  const [busy, setBusy] = useState(false);

  const refresh = useCallback(async () => {
    const supported = await getNotificationCapability();
    setCapability(supported);
    setStatus(supported.supported ? await getNotificationPermission() : 'unavailable');
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const handleAllow = async () => {
    setBusy(true);
    try {
      const granted = await requestNotificationPermission();
      await refresh();
      if (!granted) {
        toast.error('Notifications were not allowed', {
          description: 'You can turn them on in System Settings → Notifications.',
        });
      }
    } finally {
      setBusy(false);
    }
  };

  const handleTest = async () => {
    setBusy(true);
    try {
      const sent = await notify({
        title: 'Nixon',
        body: 'Notifications are working. This is what a meeting alert will look like.',
        category: CATEGORY_PLAIN,
        id: 'nixon-test',
      });
      if (!sent) {
        toast.error('Could not send the test notification');
      }
    } finally {
      setBusy(false);
    }
  };

  const allowed = status === 'authorized' || status === 'provisional';

  return (
    <>
    <SettingsRow
      label="Notifications"
      description="Alerts about meetings about to start, and calls Nixon spots, shown by macOS even when Nixon is behind another window."
      control={
        <div className="flex w-64 items-center justify-end gap-2" data-testid="notification-permission">
          {status === null ? (
            <Loader2 className="h-4 w-4 animate-spin text-muted-foreground" aria-label="Checking" />
          ) : !capability?.supported ? (
            <span className="text-right text-xs text-muted-foreground">
              {capability?.reason ?? 'Not available in this build.'}
            </span>
          ) : allowed ? (
            <>
              <span className="inline-flex items-center gap-1.5 text-sm text-success">
                <Check className="h-3.5 w-3.5" aria-hidden />
                Allowed
              </span>
              <Button variant="outline" size="sm" onClick={handleTest} disabled={busy}>
                <Bell className="mr-1.5 h-3.5 w-3.5" aria-hidden />
                Send a test
              </Button>
            </>
          ) : status === 'denied' ? (
            <>
              <span className="text-sm text-muted-foreground">Turned off</span>
              <Button variant="outline" size="sm" onClick={() => void openNotificationSettings()}>
                Open System Settings
              </Button>
            </>
          ) : (
            <Button variant="outline" size="sm" onClick={handleAllow} disabled={busy}>
              Allow…
            </Button>
          )}
        </div>
      }
    />
    {/* Owner feedback 2026-09-21: "the 'meeting starts now - join & record' should be
        persistent, but all others transient." macOS has no per-notification control — the
        Banner/Alert choice is one app-wide toggle, and the entitlement that would override
        it is Apple's to grant. So Nixon runs as Alerts and takes back the ones that should
        not have stayed, which means the one thing the user has to do is pick Alerts. Shown
        only once notifications actually work; before that it is advice about a thing that
        cannot happen yet. */}
    {allowed && capability?.supported && (
      <SettingsNote tone="info">
        For the &quot;meeting is starting&quot; alert to wait for you, set Nixon to{' '}
        <strong className="font-semibold">Alerts</strong> in System Settings → Notifications.
        Nixon clears the rest by itself after a few seconds.{' '}
        <button
          type="button"
          onClick={() => void openNotificationSettings()}
          className="font-semibold text-brand underline-offset-2 hover:underline focus:outline-none focus-visible:ring-2 focus-visible:ring-ring"
        >
          Open Notifications
        </button>
      </SettingsNote>
    )}
    </>
  );
}
