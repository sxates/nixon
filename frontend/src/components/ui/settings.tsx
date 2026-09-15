'use client';

import * as React from 'react';
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
 * engraved caps section label — the Nixon Faceplate/Deck geometry (`rounded-[3px]`,
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
  children,
  id,
  className,
}: SettingsSectionProps) {
  const generatedId = React.useId();
  const labelId = id ?? `settings-section-${generatedId}`;
  return (
    <section aria-labelledby={labelId} className={cn('space-y-3', className)}>
      <div>
        <h2 id={labelId} className="u-section-label">
          {title}
        </h2>
        {description ? <p className="u-meta mt-1">{description}</p> : null}
      </div>
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
    <div className={cn('rounded-[3px] border border-border bg-card px-4', className)}>
      {children}
    </div>
  );
}

export interface SettingsRowProps {
  /** The setting's name — one `text-sm font-medium` line. */
  label: React.ReactNode;
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
  className,
}: SettingsRowProps) {
  const labelClass = 'text-sm font-medium text-foreground';
  return (
    <div className={cn('border-b border-border py-3 last:border-b-0', className)}>
      <div
        className={cn(
          'flex items-center justify-between gap-4',
          align === 'start' && 'items-start',
        )}
      >
        <div className="min-w-0 flex-1 pr-4">
          {htmlFor ? (
            <label htmlFor={htmlFor} className={labelClass}>
              {label}
            </label>
          ) : (
            <div className={labelClass}>{label}</div>
          )}
          {description ? (
            <div className="text-sm text-muted-foreground">{description}</div>
          ) : null}
        </div>
        {control ? <div className="flex-none">{control}</div> : null}
      </div>
      {children ? <div className="mt-3">{children}</div> : null}
    </div>
  );
}

export type SettingsNoteTone = 'info' | 'warn' | 'muted';

const NOTE_TONE_CLASS: Record<SettingsNoteTone, string> = {
  info: 'border-brand/30 bg-brand/10',
  warn: 'border-record/30 bg-record/10',
  muted: 'bg-muted',
};

export interface SettingsNoteProps {
  tone?: SettingsNoteTone;
  children?: React.ReactNode;
  className?: string;
  /** e.g. `status` for a live banner. */
  role?: string;
}

/** The callout box used for banners and explanatory notes. */
export function SettingsNote({
  tone = 'muted',
  children,
  className,
  role,
}: SettingsNoteProps) {
  return (
    <div
      role={role}
      data-tone={tone}
      className={cn(
        'rounded-[3px] border border-border p-4 text-sm text-foreground',
        NOTE_TONE_CLASS[tone],
        className,
      )}
    >
      {children}
    </div>
  );
}
