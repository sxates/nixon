'use client';

import * as React from 'react';

import { cn } from '@/lib/utils';

export interface PageHeaderProps {
  /** The page title. A node, not just a string — person details passes an inline editor. */
  title: React.ReactNode;
  /** One line of context under the title, in `.u-meta`. */
  subtitle?: React.ReactNode;
  /** Right-hand cluster (search, primary action, overflow menu). */
  actions?: React.ReactNode;
  /** Optional control centred in the header between the title and the actions (e.g. the
   *  Meetings view switch). */
  afterTitle?: React.ReactNode;
  /** Never children — the header owns its own layout. */
  children?: never;
  className?: string;
}

/**
 * The one page header (specs/0057 Task 2). Every top-level screen — Today, All
 * meetings, People, Tasks, Ask AI, Settings, Saved question, Person details —
 * renders its title block through this so the type scale, the padding and the
 * actions cluster are identical everywhere instead of eight near-copies.
 */
export function PageHeader({ title, subtitle, actions, afterTitle, className }: PageHeaderProps) {
  const titleBlock = (
    <>
      <h1 className="font-display text-[22px] font-semibold leading-[1.1] tracking-[-0.011em] text-foreground">
        {title}
      </h1>
      {subtitle ? <p className="u-meta mt-1 truncate">{subtitle}</p> : null}
    </>
  );
  const actionsBlock = actions ? (
    <div className="flex flex-shrink-0 items-center gap-2.5">{actions}</div>
  ) : null;

  if (afterTitle) {
    // Three columns: title left, the control centred on the header, actions right. The
    // two `1fr` sides keep the centre column truly centred whatever their widths.
    return (
      <header
        className={cn(
          'grid flex-shrink-0 grid-cols-[minmax(0,1fr)_auto_minmax(0,1fr)] items-start gap-4 px-4 min-[900px]:px-7 pb-4 pt-7',
          className
        )}
      >
        <div className="min-w-0">{titleBlock}</div>
        <div className="flex items-center self-center">{afterTitle}</div>
        <div className="flex justify-end">{actionsBlock}</div>
      </header>
    );
  }

  return (
    <header
      className={cn(
        'flex flex-shrink-0 items-start justify-between gap-4 px-4 min-[900px]:px-7 pb-4 pt-7',
        className
      )}
    >
      {/* `flex-1`, not just `min-w-0`: without it this box takes its content width and
          then shrinks, so a two-word name wraps while the rest of the row sits empty
          (owner report on the person page, specs/0067). Actions stay `flex-shrink-0`, so
          the title claims the free space and wraps only when there genuinely is none. */}
      <div className="min-w-0 flex-1">{titleBlock}</div>
      {actionsBlock}
    </header>
  );
}
