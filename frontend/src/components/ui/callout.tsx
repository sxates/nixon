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
 *
 * `size`: `default` (`px-4 py-3 text-sm`) or `compact` (`px-3 py-2 text-xs`) for a one-line
 * status strip under a toolbar. `lead`: a spinner or a `StatusDot` before the text, centred
 * on its first line — the Callout's tone carries the status, so never a warning glyph.
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
  /** `default` = `px-4 py-3 text-sm`; `compact` = `px-3 py-2 text-xs` (status strips). */
  size?: CalloutSize;
  /** A spinner or status dot before the content. */
  lead?: React.ReactNode;
  /**
   * Where `action` sits: `center` (default) beside the whole box, or `start` on the title
   * line, with the body running the full width under it — for a tall callout whose body
   * holds rows with their own buttons.
   */
  actionAlign?: 'center' | 'start';
}

export const CALLOUT_SIZES = {
  default: 'px-4 py-3 text-sm',
  compact: 'px-3 py-2 text-xs',
} as const;

export type CalloutSize = keyof typeof CALLOUT_SIZES;

export const Callout = React.forwardRef<HTMLDivElement, CalloutProps>(function Callout(
  {
    tone = 'info',
    title,
    action,
    size = 'default',
    lead,
    actionAlign = 'center',
    className,
    children,
    role,
    ...props
  },
  ref,
) {
  const structured = title != null || action != null || lead != null;
  return (
    <div
      ref={ref}
      // An error is announced; other tones are passive unless the caller asks (`status`).
      role={role ?? (tone === 'error' ? 'alert' : undefined)}
      data-tone={tone}
      data-size={size}
      className={cn(
        'relative w-full rounded-lg border text-foreground',
        CALLOUT_SIZES[size],
        CALLOUT_TONES[tone],
        structured && cn('flex items-start', size === 'compact' ? 'gap-2' : 'gap-3'),
        className,
      )}
      {...props}
    >
      {structured ? (
        <>
          {lead != null ? (
            // `h-[1lh]`: one line box of the Callout's own text size, so the lead centres on
            // the first line whether the body wraps or not.
            <div data-callout-lead="" className="flex h-[1lh] flex-none items-center">
              {lead}
            </div>
          ) : null}
          {actionAlign === 'start' && title != null ? (
            // Title line with the action on it; the body runs the full width underneath
            // (a tall body of rows with their own keys must not share a column with it).
            <div className="min-w-0 flex-1">
              <div className="flex items-center justify-between gap-3">
                <div className="font-medium text-foreground">{title}</div>
                {action != null ? (
                  <div data-callout-action="" className="flex flex-none items-center gap-2">
                    {action}
                  </div>
                ) : null}
              </div>
              {children != null ? <div className="mt-0.5 text-xs text-muted-foreground">{children}</div> : null}
            </div>
          ) : (
            <>
              <div className="min-w-0 flex-1">
                {title != null ? <div className="font-medium text-foreground">{title}</div> : null}
                {children != null ? (
                  <div className={cn(title != null && 'mt-0.5 text-xs text-muted-foreground')}>{children}</div>
                ) : null}
              </div>
              {action != null ? (
                <div
                  data-callout-action=""
                  className={cn('flex flex-none items-center gap-2', actionAlign === 'start' ? 'self-start' : 'self-center')}
                >
                  {action}
                </div>
              ) : null}
            </>
          )}
        </>
      ) : (
        children
      )}
    </div>
  );
});
