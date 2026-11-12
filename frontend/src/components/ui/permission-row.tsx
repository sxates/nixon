/**
 * PermissionStatus and PermissionRow (docs/design-system.md §2) — one shape for "is Nixon
 * allowed to …?" in Settings (Audio, Notifications), onboarding and the PermissionsModal.
 *
 * `PermissionStatus` is a StatusDot and a word, never a glyph or a second check in a circle:
 *   granted  → green dot, "Allowed"
 *   denied   → red (destructive) dot, "Not allowed"
 *   silent   → muted dot, "Not yet" (the audio tap opened but only heard silence; macOS may
 *              ask again on the first recording — specs/0061 W3)
 *   checking → spinner, "Checking…"
 * `label` overrides the word.
 *
 * `PermissionRow` is title + description on the left, the status and an `action` on the
 * right. The action is the caller's `Button variant="outline" size="sm"` ("Allow…",
 * "Open System Settings"); pass nothing once granted. `variant="card"` frames the row as a
 * bordered card (onboarding, the modal); the default is a flat row for a `SettingsGroup`.
 */

import * as React from 'react';
import { Spinner } from '@/components/ui/spinner';
import { StatusDot, type StatusDotTone } from '@/components/ui/status-dot';
import { cn } from '@/lib/utils';

export type PermissionState = 'granted' | 'denied' | 'silent' | 'checking';

const STATE: Record<Exclude<PermissionState, 'checking'>, { tone: StatusDotTone; text: string; word: string }> = {
  granted: { tone: 'success', text: 'text-success', word: 'Allowed' },
  denied: { tone: 'destructive', text: 'text-destructive', word: 'Not allowed' },
  silent: { tone: 'muted', text: 'text-muted-foreground', word: 'Not yet' },
};

export interface PermissionStatusProps {
  state: PermissionState;
  /** Replaces the default word. */
  label?: React.ReactNode;
  className?: string;
}

export function PermissionStatus({ state, label, className }: PermissionStatusProps) {
  if (state === 'checking') {
    return (
      <span
        role="status"
        data-state={state}
        className={cn('inline-flex items-center gap-1.5 text-sm text-muted-foreground', className)}
      >
        <Spinner />
        {label ?? 'Checking…'}
      </span>
    );
  }
  const s = STATE[state];
  return (
    <span role="status" data-state={state} className={cn('inline-flex items-center gap-1.5 text-sm', s.text, className)}>
      <StatusDot tone={s.tone} />
      {label ?? s.word}
    </span>
  );
}

export interface PermissionRowProps {
  title: React.ReactNode;
  description?: React.ReactNode;
  state: PermissionState;
  /** Replaces the status word. */
  statusLabel?: React.ReactNode;
  /** The row's button. Omit when there's nothing to do (granted). */
  action?: React.ReactNode;
  /** `row` (default): flat, for a SettingsGroup. `card`: a bordered card. */
  variant?: 'row' | 'card';
  className?: string;
}

export function PermissionRow({
  title,
  description,
  state,
  statusLabel,
  action,
  variant = 'row',
  className,
}: PermissionRowProps) {
  return (
    <div
      data-state={state}
      className={cn(
        'flex items-center gap-4',
        variant === 'card' ? 'rounded-lg border border-border bg-card px-4 py-3' : 'py-3',
        className,
      )}
    >
      <div className="min-w-0 flex-1">
        <div className="text-sm font-medium text-foreground">{title}</div>
        {description != null ? <div className="text-xs text-muted-foreground">{description}</div> : null}
      </div>
      <div className="flex flex-none items-center gap-3">
        <PermissionStatus state={state} label={statusLabel} />
        {action}
      </div>
    </div>
  );
}
