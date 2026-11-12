/**
 * Callout (docs/design-system.md §2) — the inline notice: a banner, an explanatory note, an
 * error box. `rounded-lg border px-4 py-3 text-sm`, tinted with the named washes.
 *
 * Tones:
 *   - `info`  — amber wash + amber edge: "worth knowing" (the Settings note dialect).
 *   - `warn`  — the same wash under a full-strength amber edge: something needs attention
 *               (a permission, a Bluetooth output, a stage that didn't finish). Never red:
 *               red is recording only.
 *   - `error` — destructive wash + edge, `role="alert"`. Replaces the hand-made
 *               `border-destructive/30 bg-destructive/5` boxes.
 *   - `muted` — a flat `bg-muted` note.
 *
 * Structured use: `title` (one `font-medium` line), children (the body, muted under a
 * title), and an `action` slot on the right (primary action first, `Button size="sm"`).
 * With neither `title` nor `action` the children render straight into the box, so
 * `ui/alert`'s `[&>svg]` icon layout and `SettingsNote` keep working as thin wrappers.
 */

import * as React from 'react';
import { cn } from '@/lib/utils';

export const CALLOUT_TONES = {
  info: 'border-brand-edge bg-brand-wash',
  warn: 'border-brand bg-brand-wash',
  error: 'border-destructive-edge bg-destructive-wash',
  muted: 'border-border bg-muted',
} as const;

export type CalloutTone = keyof typeof CALLOUT_TONES;

export interface CalloutProps extends Omit<React.HTMLAttributes<HTMLDivElement>, 'title'> {
  tone?: CalloutTone;
  /** One-line heading. */
  title?: React.ReactNode;
  /** Right-aligned controls (primary first). */
  action?: React.ReactNode;
}

export const Callout = React.forwardRef<HTMLDivElement, CalloutProps>(function Callout(
  { tone = 'info', title, action, className, children, role, ...props },
  ref,
) {
  const structured = title != null || action != null;
  return (
    <div
      ref={ref}
      // An error is announced; other tones are passive unless the caller asks (`status`).
      role={role ?? (tone === 'error' ? 'alert' : undefined)}
      data-tone={tone}
      className={cn(
        'relative w-full rounded-lg border px-4 py-3 text-sm text-foreground',
        CALLOUT_TONES[tone],
        structured && 'flex items-start gap-3',
        className,
      )}
      {...props}
    >
      {structured ? (
        <>
          <div className="min-w-0 flex-1">
            {title != null ? <div className="font-medium text-foreground">{title}</div> : null}
            {children != null ? (
              <div className={cn(title != null && 'mt-0.5 text-xs text-muted-foreground')}>{children}</div>
            ) : null}
          </div>
          {action != null ? (
            <div className="flex flex-none items-center gap-2 self-center">{action}</div>
          ) : null}
        </>
      ) : (
        children
      )}
    </div>
  );
});
