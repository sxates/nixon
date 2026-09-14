'use client';

/**
 * Requests OS notification permission once at app startup (spec 0008, #4).
 *
 * Why a dedicated mounted component: previously notification permission was only
 * requested lazily inside `osNotification.notify()` — i.e. mid-meeting, while
 * Nixon is backgrounded and the user is in Zoom, where the macOS permission
 * dialog is easy to miss. For a fresh bundle id the plugin's `isPermissionGranted`
 * is not auto-granted, so `notify()` would silently return false and the Zoom
 * "record this?" prompt would be lost. Asking up front, on launch, lets the user
 * grant it before any meeting starts.
 *
 * Best-effort and side-effect-only: renders nothing. On a bare `cargo run` dev
 * binary (no plugin) it no-ops. Mounted post-onboarding inside the provider tree.
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
