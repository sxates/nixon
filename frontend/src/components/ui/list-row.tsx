'use client';

/**
 * ListRow and LinkRow (docs/design-system.md §2) — one row in a list of meetings, people,
 * prompts or sources.
 *
 * `ListRow` slots: `lead` (an avatar, a dot, a spine), `title` (`.u-row-title`), `meta`
 * (`.u-row-meta`, under the title), `trailing` (badges, a duration, a star toggle) and
 * `menu` (a `MenuButton` inside a `DropdownMenu`). With `onClick` the lead/title/meta area
 * becomes a real `<button>` with the focus ring; `trailing` and `menu` sit OUTSIDE it, so a
 * star toggle or overflow menu is never a button nested in a button. `menuReveal="hover"`
 * hides the menu until the row is hovered or focused (it stays visible while open).
 *
 * `LinkRow` is the compact navigational row: title and meta on one baseline, then a
 * chevron; the title and chevron light to brand on hover. For "open this meeting" lists
 * (Ask sources, a person's meetings).
 */

import * as React from 'react';
import { DeckIcon } from '@/components/ui/deck-icon';
import { ChevronRight } from '@/components/ui/icons';
import { cn } from '@/lib/utils';

export interface ListRowProps {
  lead?: React.ReactNode;
  title: React.ReactNode;
  meta?: React.ReactNode;
  trailing?: React.ReactNode;
  menu?: React.ReactNode;
  /** Makes the main area a button. */
  onClick?: () => void;
  /** Accessible name for the main button when the visible title isn't enough. */
  'aria-label'?: string;
  /** Native tooltip on the main area ("Open this meeting"). */
  hint?: string;
  /** Selected row: the brand wash. */
  selected?: boolean;
  /** `always` (default) or `hover`: the menu appears on hover / focus-within / open. */
  menuReveal?: 'always' | 'hover';
  /** A hairline under the row (dense tables of rows); off for stacked rows. */
  ruled?: boolean;
  className?: string;
}

export function ListRow({
  lead,
  title,
  meta,
  trailing,
  menu,
  onClick,
  'aria-label': ariaLabel,
  hint,
  selected = false,
  menuReveal = 'always',
  ruled = false,
  className,
}: ListRowProps) {
  const body = (
    <>
      {lead != null ? <span className="flex flex-none items-center">{lead}</span> : null}
      <span className="min-w-0 flex-1">
        <span className="u-row-title block truncate">{title}</span>
        {meta != null ? <span className="u-row-meta block truncate">{meta}</span> : null}
      </span>
    </>
  );
  const mainClass = 'flex min-w-0 flex-1 items-center gap-3 px-3 py-2 text-left';
  return (
    <div
      data-selected={selected || undefined}
      className={cn(
        'group/row flex w-full items-center rounded-md transition-colors',
        selected ? 'bg-brand-wash' : 'hover:bg-accent',
        ruled && 'rounded-none border-b border-rule',
        className,
      )}
    >
      {onClick ? (
        <button
          type="button"
          onClick={onClick}
          aria-label={ariaLabel}
          title={hint}
          aria-current={selected || undefined}
          className={cn(
            mainClass,
            'rounded-md focus:outline-none focus-visible:ring-2 focus-visible:ring-ring',
          )}
        >
          {body}
        </button>
      ) : (
        <div className={mainClass} title={hint}>
          {body}
        </div>
      )}
      {trailing != null ? (
        <div className={cn('flex flex-none items-center gap-2', menu == null && 'pr-3')}>{trailing}</div>
      ) : null}
      {menu != null ? (
        <div
          data-row-menu=""
          className={cn(
            'flex flex-none items-center pl-1 pr-2',
            menuReveal === 'hover' &&
              'opacity-0 transition-opacity focus-within:opacity-100 group-hover/row:opacity-100 has-[[data-state=open]]:opacity-100',
          )}
        >
          {menu}
        </div>
      ) : null}
    </div>
  );
}

export interface LinkRowProps {
  title: React.ReactNode;
  /** A date or count on the same baseline. */
  meta?: React.ReactNode;
  onClick: () => void;
  /** De-emphasised row (an uncited source): muted, regular weight title. */
  muted?: boolean;
  /** Native tooltip ("Open this meeting"). */
  hint?: string;
  className?: string;
}

export function LinkRow({ title, meta, onClick, muted = false, hint, className }: LinkRowProps) {
  return (
    <button
      type="button"
      onClick={onClick}
      title={hint}
      className={cn(
        'group/link flex w-full items-baseline gap-2 rounded-md px-2 py-1.5 text-left transition-colors',
        'focus:outline-none focus-visible:ring-2 focus-visible:ring-ring',
        className,
      )}
    >
      <span
        className={cn(
          'min-w-0 truncate group-hover/link:text-brand',
          muted ? 'text-body text-muted-foreground' : 'u-row-title',
        )}
      >
        {title}
      </span>
      {meta != null ? <span className="u-row-meta flex-none">{meta}</span> : null}
      <DeckIcon
        icon={ChevronRight}
        size={12}
        className="flex-none self-center text-muted-foreground group-hover/link:text-brand"
      />
    </button>
  );
}
