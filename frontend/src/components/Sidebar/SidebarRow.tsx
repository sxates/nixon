'use client';

import React from 'react';
import type { LucideIcon } from '@/components/ui/icons';
import { Badge } from '@/components/ui/badge';
import { DeckIcon } from '@/components/ui/deck-icon';
import { cn } from '@/lib/utils';
import { SIDEBAR_GLYPH, SIDEBAR_ICON_SLOT, SIDEBAR_LABEL, SIDEBAR_ROW } from './row';

/**
 * The leading-element column (specs/0069 W1). `name` is published as
 * `data-sidebar-slot` so the parity test can assert that both states render the same
 * columns in the same order — jsdom has no layout, so the structure is the contract.
 */
export function IconSlot({ name, children }: { name: string; children: React.ReactNode }) {
  return (
    <span data-sidebar-slot={name} className={SIDEBAR_ICON_SLOT}>
      {children}
    </span>
  );
}

/** The 1.5px amber bar that indexes the active row. Same x in both states. */
export function IndexBar({ active }: { active: boolean }) {
  return (
    <span
      aria-hidden
      className={cn(
        'absolute top-2 bottom-2 left-1.5 w-[1.5px] [transition-property:background-color] [transition-duration:120ms]',
        active
          ? 'bg-brand shadow-[0_0_6px_-1px_hsl(var(--brand)/0.6)]' // ui-gate: allow the index bar is a lit amber lamp; the glow is its light
          : 'bg-transparent',
      )}
    />
  );
}

/**
 * One clickable destination. Collapsed and expanded differ by exactly one thing: whether
 * the label element exists. Everything positional lives in the shared constants.
 */
export function NavRow({
  slot,
  icon,
  label,
  collapsed,
  active = false,
  onClick,
  indexBar = true,
  badge,
}: {
  slot: string;
  icon: LucideIcon;
  label: string;
  collapsed: boolean;
  active?: boolean;
  onClick: () => void;
  indexBar?: boolean;
  /** A count chip (the To-do row's inbox count); hidden at 0. Collapsed, it perches on the glyph. */
  badge?: number;
}) {
  return (
    <button // ui-gate: allow sidebar row: the shared row.ts geometry (64px icon column, no padding); ListRow's padding and radius would move the rail glyphs
      type="button"
      data-sidebar-row
      onClick={onClick}
      aria-label={label}
      title={collapsed ? label : undefined}
      aria-current={active ? 'page' : undefined}
      className={cn(
        SIDEBAR_ROW,
        'relative transition-colors hover:bg-key focus:outline-none focus-visible:ring-2 focus-visible:ring-ring',
        active ? 'text-foreground' : 'text-engrave',
      )}
    >
      {indexBar && <IndexBar active={active} />}
      <IconSlot name={slot}>
        <DeckIcon icon={icon} size={SIDEBAR_GLYPH} />
      </IconSlot>
      {!collapsed && <span className={SIDEBAR_LABEL}>{label}</span>}
      {badge ? (
        <Badge
          tone="brand"
          data-sidebar-badge
          title={`${badge} in your inbox`}
          className={collapsed ? 'absolute right-2.5 top-1 px-1 py-0' : 'mr-3.5'}
        >
          {badge}
        </Badge>
      ) : null}
    </button>
  );
}
