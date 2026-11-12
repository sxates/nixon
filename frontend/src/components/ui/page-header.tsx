'use client';

import * as React from 'react';

import { DeckIcon } from '@/components/ui/deck-icon';
import { IconButton } from '@/components/ui/icon-button';
import { ArrowLeft } from '@/components/ui/icons';
import { PAGE_GUTTER } from '@/lib/page-width';
import { cn } from '@/lib/utils';

export interface PageHeaderBack {
  /** Where back goes ("People"). Becomes the button's tooltip and accessible name. */
  label: string;
  onClick: () => void;
}

export interface PageHeaderProps {
  /** The page title. A node, not just a string — person details passes an inline editor. */
  title: React.ReactNode;
  /**
   * Context under the title, in `.u-meta`. Rendered OUTSIDE the h1, so an editor passed
   * here (a person's role) keeps its own weight instead of inheriting the title's. A
   * string renders in a `<p>`; any other node in a `<div>` (an editor is not phrasing
   * content).
   */
  subtitle?: React.ReactNode;
  /** A back button left of the title: an `IconButton` with the left arrow. */
  back?: PageHeaderBack;
  /** Right-hand cluster (search, primary action, overflow menu). */
  actions?: React.ReactNode;
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
export function PageHeader({ title, subtitle, back, actions, className }: PageHeaderProps) {
  const hasSubtitle = subtitle !== undefined && subtitle !== null && subtitle !== false && subtitle !== '';
  const subtitleIsText = typeof subtitle === 'string' || typeof subtitle === 'number';
  return (
    <header
      className={cn('flex flex-shrink-0 items-start justify-between gap-4 pb-4 pt-7', PAGE_GUTTER, className)}
    >
      {/* `flex-1`, not just `min-w-0`: without it this box takes its content width and
          then shrinks, so a two-word name wraps while the rest of the row sits empty
          (owner report on the person page, specs/0067). Actions stay `flex-shrink-0`, so
          the title claims the free space and wraps only when there genuinely is none. */}
      <div className="flex min-w-0 flex-1 items-start gap-2">
        {back ? (
          // -mt-0.5 centres the 28px key on the title's 24px first line.
          <IconButton
            variant="ghost"
            size="sm"
            label={back.label}
            onClick={back.onClick}
            icon={<DeckIcon icon={ArrowLeft} size={16} />}
            className="-ml-1.5 -mt-0.5"
          />
        ) : null}
        <div className="min-w-0 flex-1">
          <h1 className="u-page-title">{title}</h1>
          {hasSubtitle ? (
            subtitleIsText ? (
              <p className="u-meta mt-1 truncate">{subtitle}</p>
            ) : (
              <div className="u-meta mt-1 min-w-0">{subtitle}</div>
            )
          ) : null}
        </div>
      </div>
      {actions ? <div className="flex flex-shrink-0 items-center gap-2.5">{actions}</div> : null}
    </header>
  );
}
