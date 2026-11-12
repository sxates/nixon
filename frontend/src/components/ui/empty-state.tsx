/**
 * EmptyState and PageLoading (docs/design-system.md §2) — the "nothing yet" and
 * "loading" shapes (a run in progress is `RunStatus`). No icon discs: the 48–56px glyph circles above the
 * old empty states are gone (§Icons). An error is `Callout tone="error"`, not one of these.
 */

import * as React from 'react';
import { Panel } from '@/components/ui/panel';
import { Spinner } from '@/components/ui/spinner';
import { cn } from '@/lib/utils';

export interface EmptyStateProps {
  title: React.ReactNode;
  body?: React.ReactNode;
  /** One `Button` (or a pair, primary first). */
  action?: React.ReactNode;
  /**
   * `plain` (default) sits centred in the page or pane; `panel` is a dashed placeholder
   * card for an empty section among panels (an empty brief, no voice samples yet).
   */
  variant?: 'plain' | 'panel';
  /** Heading level for the title. Defaults to `h2`. */
  titleAs?: 'h2' | 'h3' | 'h4';
  className?: string;
}

export function EmptyState({
  title,
  body,
  action,
  variant = 'plain',
  titleAs: Heading = 'h2',
  className,
}: EmptyStateProps) {
  const content = (
    <>
      <Heading className="text-lead font-semibold text-foreground">{title}</Heading>
      {body != null ? <div className="mt-1 max-w-sm text-sm text-muted-foreground">{body}</div> : null}
      {action != null ? <div className="mt-4 flex items-center justify-center gap-2">{action}</div> : null}
    </>
  );
  if (variant === 'panel') {
    return (
      <Panel
        variant="dashed"
        data-empty-state="panel"
        className={cn('flex flex-col items-center px-4 py-6 text-center', className)}
      >
        {content}
      </Panel>
    );
  }
  return (
    <div
      data-empty-state="plain"
      className={cn('flex flex-col items-center justify-center py-12 text-center', className)}
    >
      {content}
    </div>
  );
}

export interface PageLoadingProps {
  /** Shown beside the spinner and used as its accessible name. Defaults to "Loading". */
  label?: string;
  /** Hide the label visually (it stays the accessible name). */
  hideLabel?: boolean;
  /** `page` fills the page under the rail (`h-page`); `pane` is a 256px block in a pane. */
  fill?: 'page' | 'pane';
  className?: string;
}

/** A centred spinner (and label) while a page or pane loads. */
export function PageLoading({ label, hideLabel = false, fill = 'page', className }: PageLoadingProps) {
  const name = label ?? 'Loading';
  return (
    <div
      role="status"
      aria-label={name}
      data-fill={fill}
      className={cn(
        'flex items-center justify-center gap-2 text-muted-foreground',
        fill === 'page' ? 'h-page' : 'h-64',
        className,
      )}
    >
      <Spinner size="md" />
      {label && !hideLabel ? <span className="text-sm">{label}</span> : null}
    </div>
  );
}
