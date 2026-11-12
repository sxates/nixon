/**
 * Panel (docs/design-system.md §2) — the card: `rounded-lg border bg-card shadow-card`,
 * padded `px-4 py-3` (`size="document"` → `p-6` for a summary, an answer, a brief).
 *
 * The header is optional. With a `title` it is an engraved `.u-section-label` heading, an
 * optional `count` chip (the same chip the tab badge uses) and an `action` slot pushed to
 * the right, and the panel becomes a `<section>` named by that heading. No icons on the
 * header (§Icons): the Users/ListTodo glyphs on the old card headers are gone.
 *
 * `variant="dashed"` is the placeholder card — a dashed hairline on a half-strength card,
 * no shadow — for "nothing here yet" content inside a page that otherwise has panels.
 */

import * as React from 'react';
import { cn } from '@/lib/utils';

export const PANEL_VARIANTS = {
  solid: 'border-border bg-card shadow-card',
  dashed: 'border-dashed border-border bg-card/50',
} as const;

export const PANEL_SIZES = {
  default: 'px-4 py-3',
  document: 'p-6',
} as const;

export type PanelVariant = keyof typeof PANEL_VARIANTS;
export type PanelSize = keyof typeof PANEL_SIZES;

export interface PanelProps extends Omit<React.HTMLAttributes<HTMLElement>, 'title'> {
  /** Engraved caps heading. Makes the panel a labelled `<section>`. */
  title?: React.ReactNode;
  /** A count beside the title (rendered as a small chip). `0` is shown; `null` is not. */
  count?: number | null;
  /** Right-aligned header control (a `Button`, `IconButton`, `MenuButton`). */
  action?: React.ReactNode;
  variant?: PanelVariant;
  size?: PanelSize;
  /** Heading level for the title. Defaults to `h2`. */
  titleAs?: 'h2' | 'h3' | 'h4';
}

export const Panel = React.forwardRef<HTMLElement, PanelProps>(function Panel(
  {
    title,
    count,
    action,
    variant = 'solid',
    size = 'default',
    titleAs: Heading = 'h2',
    className,
    children,
    id,
    ...props
  },
  ref,
) {
  const generatedId = React.useId();
  const hasHeader = title != null || action != null;
  const titleId = `${id ?? `panel-${generatedId}`}-title`;
  const Root = title != null ? 'section' : 'div';
  return (
    <Root
      ref={ref as React.Ref<HTMLDivElement>}
      id={id}
      data-variant={variant}
      data-size={size}
      aria-labelledby={title != null ? titleId : undefined}
      className={cn('rounded-lg border', PANEL_VARIANTS[variant], PANEL_SIZES[size], className)}
      {...props}
    >
      {hasHeader ? (
        <PanelHeader title={title} titleId={titleId} titleAs={Heading} count={count} action={action} />
      ) : null}
      {children}
    </Root>
  );
});

export interface PanelHeaderProps {
  title?: React.ReactNode;
  /** Id for the heading, so a surrounding region can be named by it. */
  titleId?: string;
  titleAs?: 'h2' | 'h3' | 'h4';
  count?: number | null;
  action?: React.ReactNode;
  className?: string;
}

/**
 * The Panel's header row on its own: engraved heading, count chip, right-aligned action.
 * For a section that is deliberately flat (the Prep tab's brief has no card) but must
 * share the heading rhythm of the panels beside it.
 */
export function PanelHeader({ title, titleId, titleAs: Heading = 'h2', count, action, className }: PanelHeaderProps) {
  return (
    <div className={cn('mb-2 flex min-h-6 items-center gap-2', className)} data-panel-header="">
      {title != null ? (
        <Heading id={titleId} className="u-section-label min-w-0 truncate">
          {title}
        </Heading>
      ) : null}
      {count != null ? (
        <span
          data-panel-count=""
          className="flex-none rounded-sm bg-muted px-1 text-caption font-medium tabular-nums text-muted-foreground"
        >
          {count}
        </span>
      ) : null}
      {action != null ? <div className="ml-auto flex flex-none items-center gap-1">{action}</div> : null}
    </div>
  );
}
