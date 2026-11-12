'use client';

/**
 * ConfirmDialog (docs/design-system.md §2) — "are you sure?" on `Dialog`: a title, a body,
 * Cancel and the confirm button. Replaces the seven copies (Clear voiceprints, Retention
 * change, Move recordings, Delete prompt, Save one-off, Calendar disconnect, CLI provider).
 *
 * `tone="destructive"` (default) makes the confirm the filled destructive button — the one
 * place that variant belongs; `tone="brand"` is the amber CTA for a non-destructive commit
 * (moving recordings). While `busy` the confirm shows a spinner, both buttons are disabled
 * and the dialog can't be dismissed (Escape, overlay, ×), so a half-done delete can't be
 * walked away from. `onConfirm` may return a promise; closing on success is the caller's
 * call (most close; some keep the dialog open to show an error).
 *
 * `body` is the description (named by `aria-describedby`); `children` go under it for
 * details such as a size estimate or a list of what will be removed.
 */

import * as React from 'react';
import { Button } from '@/components/ui/button';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog';
import { Spinner } from '@/components/ui/spinner';
import { cn } from '@/lib/utils';

export interface ConfirmDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  title: React.ReactNode;
  body?: React.ReactNode;
  confirmLabel: React.ReactNode;
  cancelLabel?: React.ReactNode;
  tone?: 'destructive' | 'brand';
  /** The confirm is running. */
  busy?: boolean;
  /** Disable the confirm without the busy state (e.g. not enough disk space). */
  confirmDisabled?: boolean;
  onConfirm: () => void | Promise<void>;
  children?: React.ReactNode;
  className?: string;
}

export function ConfirmDialog({
  open,
  onOpenChange,
  title,
  body,
  confirmLabel,
  cancelLabel = 'Cancel',
  tone = 'destructive',
  busy = false,
  confirmDisabled = false,
  onConfirm,
  children,
  className,
}: ConfirmDialogProps) {
  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        if (busy && !next) return;
        onOpenChange(next);
      }}
    >
      <DialogContent
        className={cn('max-w-md', className)}
        data-tone={tone}
        // No body → no description; tell Radix so rather than leaving a dangling id.
        {...(body == null ? { 'aria-describedby': undefined } : {})}
      >
        <DialogHeader>
          <DialogTitle>{title}</DialogTitle>
          {body != null ? (
            <DialogDescription asChild>
              <div>{body}</div>
            </DialogDescription>
          ) : null}
        </DialogHeader>
        {children}
        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)} disabled={busy}>
            {cancelLabel}
          </Button>
          <Button
            variant={tone === 'destructive' ? 'destructive' : 'brand'}
            onClick={() => void onConfirm()}
            disabled={busy || confirmDisabled}
            aria-busy={busy || undefined}
          >
            {busy ? <Spinner /> : null}
            {confirmLabel}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
