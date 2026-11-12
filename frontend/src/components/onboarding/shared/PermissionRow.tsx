import React from 'react';
import { Button } from '@/components/ui/button';
import { PermissionRow as UiPermissionRow, type PermissionState } from '@/components/ui/permission-row';
import { Spinner } from '@/components/ui/spinner';
import type { PermissionRowProps as OnboardingPermissionRowProps, PermissionStatus } from '@/types/onboarding';

/** specs/0061 W3: exact onboarding copy for a tap that opened but only heard silence. */
export const SILENT_AUDIO_CAPTURE_COPY = 'Not yet — macOS may ask again when you first record.';

/** The onboarding status vocabulary onto the shared PermissionStatus states. */
function toPermissionState(status: PermissionStatus): PermissionState {
  if (status === 'authorized') return 'granted';
  if (status === 'denied') return 'denied';
  if (status === 'checking') return 'checking';
  // `silent` (tap heard nothing) and `not_determined` (never asked) both read "Not yet".
  return 'silent';
}

// The decorative `icon` is gone (docs/design-system.md §Icons: no icon avatars in
// onboarding); callers no longer pass one.
type PermissionRowProps = Omit<OnboardingPermissionRowProps, 'icon'>;

/**
 * Onboarding's permission card: the shared `ui/PermissionRow` in its `card` variant, so
 * onboarding, Settings and the PermissionsModal show "allowed" the same way. The action
 * is "Enable" until granted, "Open Settings" once denied.
 */
export function PermissionRow({ title, description, status, isPending = false, onAction }: PermissionRowProps) {
  const isAuthorized = status === 'authorized';
  const isDenied = status === 'denied';

  const detail = isDenied
    ? 'Access Denied - Please grant in System Settings'
    : status === 'silent'
      ? SILENT_AUDIO_CAPTURE_COPY
      : description;

  return (
    <UiPermissionRow
      variant="card"
      title={title}
      description={detail}
      state={toPermissionState(status)}
      action={
        isAuthorized ? undefined : (
          <Button variant="outline" size="sm" onClick={onAction} disabled={isPending}>
            {isPending ? <Spinner /> : null}
            {isPending ? 'Checking...' : isDenied ? 'Open Settings' : 'Enable'}
          </Button>
        )
      }
    />
  );
}
