'use client';

import * as React from 'react';
import { Callout } from '@/components/ui/callout';
import { cn } from '@/lib/utils';

/**
 * Settings primitives — ONE vocabulary for every Settings surface.
 *
 * Before this file the Settings pages spoke three dialects: the Plan 3 "ruled row"
 * (RecordingSettings), the old shadcn `p-6 shadow-sm` card with an `h3 text-lg`
 * (PreferenceSettings, BetaSettings, SummaryModelSettings…), and ad-hoc bare labels
 * (TranscriptSettings). Same idea, three different text/box styles per tab.
 *
 * The shape that survived is the ruled row inside a bordered group card, under an
 * engraved caps section label — the Nixon Faceplate/Deck geometry (`rounded-lg`, 4px, a card,
 * token colors only, `u-section-label` reserved for SECTION titles, never row labels).
 *
 *   <SettingsSection title="Recording" description="…">
 *     <SettingsGroup>
 *       <SettingsRow label="…" description="…" control={<Switch … />} />
 *       <SettingsRow label="…" htmlFor="x" align="start">…wide control…</SettingsRow>
 *     </SettingsGroup>
 *     <SettingsNote tone="info">…</SettingsNote>
 *   </SettingsSection>
 */

export interface SettingsSectionProps {
  /** Engraved caps title (the `u-section-label` line). */
  title: React.ReactNode;
  /** One-line purpose statement under the title. */
  description?: React.ReactNode;
  /**
   * A control on the title line, right-aligned ("New prompt", "Refresh"). Use
   * `Button variant="outline" size="sm"` (or `dashed` for "+ Add …").
   */
  action?: React.ReactNode;
  children?: React.ReactNode;
  /**
   * Id for the title element. The `<section>` points at it with `aria-labelledby`,
   * so the region is named for assistive tech. Defaults to a generated id.
   */
  id?: string;
  className?: string;
}

/** A titled Settings region: engraved caps heading + optional meta line + content. */
export function SettingsSection({
  title,
  description,
  action,
  children,
  id,
  className,
}: SettingsSectionProps) {
  const generatedId = React.useId();
  const labelId = id ?? `settings-section-${generatedId}`;
  return (
    <section aria-labelledby={labelId} className={cn('space-y-3', className)}>
      {action ? (
        // A grid, so the action has its own column beside the title AND the description,
        // and `-my-2.5` takes the 32px key out of the row height: the header keeps the
        // rhythm of a section without an action (it used to stretch the title row to 32px).
        <div data-section-header="" className="grid grid-cols-[minmax(0,1fr)_auto] gap-x-3">
          <h2 id={labelId} className="u-section-label">
            {title}
          </h2>
          <div className="row-span-2 -my-2.5 flex flex-none items-center gap-2 self-start">{action}</div>
          {description ? <p className="u-meta mt-1">{description}</p> : null}
        </div>
      ) : (
        <div data-section-header="">
          <h2 id={labelId} className="u-section-label">
            {title}
          </h2>
          {description ? <p className="u-meta mt-1">{description}</p> : null}
        </div>
      )}
      {children}
    </section>
  );
}

export interface SettingsGroupProps {
  children?: React.ReactNode;
  className?: string;
}

/** The ruled-row card: rows inside it are separated by hairlines, last one unruled. */
export function SettingsGroup({ children, className }: SettingsGroupProps) {
  return (
    <div className={cn('rounded-lg border border-border bg-card px-4', className)}>
      {children}
    </div>
  );
}

export interface SettingsRowProps
  extends Omit<React.HTMLAttributes<HTMLDivElement>, 'children' | 'className'> {
  /** The setting's name — one `text-sm font-medium` line. */
  label: React.ReactNode;
  /**
   * Status beside the label on its line: an "Active" / "Default" `Badge`, a count. Keeps
   * the label's weight; the label truncates before the aside wraps.
   */
  aside?: React.ReactNode;
  /** De-emphasise the label (a source that is present but not in use). */
  muted?: boolean;
  /** What the setting does / what happens when it is on. */
  description?: React.ReactNode;
  /** Right-aligned control (Switch, Select, Button, small input). */
  control?: React.ReactNode;
  /**
   * When given, the label renders as a real `<label htmlFor>` so clicking it focuses
   * the control (and the control is named for assistive tech).
   */
  htmlFor?: string;
  /** Wide control (path picker, radiogroup, list) — rendered full-width BELOW the label. */
  children?: React.ReactNode;
  /** `start` for tall controls that should align with the first line of the label. */
  align?: 'center' | 'start';
  className?: string;
}

/** One ruled setting row. */
export function SettingsRow({
  label,
  description,
  control,
  htmlFor,
  children,
  align = 'center',
  aside,
  muted = false,
  className,
  ...rest
}: SettingsRowProps) {
  const labelClass = cn('text-sm font-medium', muted ? 'text-muted-foreground' : 'text-foreground');
  const labelEl = htmlFor ? (
    <label htmlFor={htmlFor} className={cn(labelClass, aside != null && 'min-w-0 truncate')}>
      {label}
    </label>
  ) : (
    <div className={cn(labelClass, aside != null && 'min-w-0 truncate')}>{label}</div>
  );
  return (
    <div className={cn('border-b border-border py-3 last:border-b-0', className)} {...rest}>
      <div
        className={cn(
          'flex items-center justify-between gap-4',
          align === 'start' && 'items-start',
        )}
      >
        <div className="min-w-0 flex-1 pr-4">
          {aside != null ? (
            <div className="flex min-w-0 items-center gap-2">
              {labelEl}
              <div data-settings-row-aside="" className="flex flex-none items-center gap-1.5">
                {aside}
              </div>
            </div>
          ) : (
            labelEl
          )}
          {description ? (
            <div className="text-sm text-muted-foreground">{description}</div>
          ) : null}
        </div>
        {control ? <div className="flex-none">{control}</div> : null}
      </div>
      {/* `Children.toArray` drops `false`/`null`, so a row whose conditional details are all
          off draws no empty 12px gap. */}
      {React.Children.toArray(children).length > 0 ? <div className="mt-3">{children}</div> : null}
    </div>
  );
}

export type SettingsNoteTone = 'info' | 'warn' | 'muted';

export interface SettingsNoteProps {
  tone?: SettingsNoteTone;
  children?: React.ReactNode;
  className?: string;
  /** e.g. `status` for a live banner. */
  role?: string;
}

/**
 * The Settings note — a thin alias of `Callout` (design-system wave 0b) with Callout's own
 * geometry (`rounded-lg`, `px-4 py-3`), like every other card and callout. `warn` used to be the REC
 * red wash; red is recording-only, so it is `Callout`'s amber `warn` now. New code uses
 * `Callout` directly (and `tone="error"` for a load failure).
 */
export function SettingsNote({
  tone = 'muted',
  children,
  className,
  role,
}: SettingsNoteProps) {
  return (
    <Callout tone={tone} role={role} className={className}>
      {children}
    </Callout>
  );
}
