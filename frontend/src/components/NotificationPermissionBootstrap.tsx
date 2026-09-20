'use client';

/**
 * Asks macOS for notification permission once, after onboarding (specs/0008, #4;
 * specs/0068 for why it now does something).
 *
 * The alternative is to ask at the point of first use, which sounds tidier and is worse:
 * the first use is a meeting alert firing while Nixon is in the background and the user is
 * in Zoom — exactly where a permission dialog goes unread, and the alert that prompted it
 * is lost. Mounted post-onboarding, this asks at a moment when the person is looking at
 * Nixon and has just finished granting microphone and audio-capture, without stacking a
 * third dialog on top of those two.
 *
 * `ensureNotificationPermission()` only shows the dialog when macOS has not been asked
 * before, so this is a no-op on every later launch — and on the unbundled dev binary,
 * where there is no notification centre to ask (specs/0068).
 */

import { useEffect } from 'react';
import { ensureNotificationPermission } from '@/lib/osNotification';

export default function NotificationPermissionBootstrap() {
  useEffect(() => {
    void ensureNotificationPermission()
      .then((granted) => {
        console.log('[NotificationPermissionBootstrap] notification permission granted:', granted);
      })
      .catch((err) => {
        console.warn('[NotificationPermissionBootstrap] failed to ensure permission:', err);
      });
  }, []);

  return null;
}
