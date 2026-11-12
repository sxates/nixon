'use client';

/**
 * DateNavigator (docs/design-system.md §2) — Today / prev / label / next, for the agenda,
 * week and month views.
 *
 * Today sits LEFT of the arrows: the navigator is right-aligned in its toolbar, so Today
 * appearing and disappearing (`showToday`) never moves the arrows. `leading` holds a
 * loading spinner in the same spot; `trailing` sits between the label and the next arrow
 * (the agenda's jump-to-date picker). The label is a polite live region, so stepping is
 * announced.
 */

import * as React from 'react';
import { Button } from '@/components/ui/button';
import { DeckIcon } from '@/components/ui/deck-icon';
import { IconButton } from '@/components/ui/icon-button';
import { ChevronLeft, ChevronRight } from '@/components/ui/icons';
import { cn } from '@/lib/utils';

export interface DateNavigatorProps {
  /** "Tue, Nov 11", "Nov 10 – 16", "November 2026". */
  label: React.ReactNode;
  onPrev: () => void;
  onNext: () => void;
  /** Accessible names for the arrows. */
  prevLabel?: string;
  nextLabel?: string;
  onToday?: () => void;
  /** Show the Today button (only meaningful with `onToday`). Defaults to true. */
  showToday?: boolean;
  /** Left of Today (a spinner while the range loads). */
  leading?: React.ReactNode;
  /** Between the label and the next arrow (a date picker). */
  trailing?: React.ReactNode;
  /** Width for the label, so the arrows don't move as it changes (`min-w-[9.5rem]`). */
  labelClassName?: string;
  className?: string;
}

export function DateNavigator({
  label,
  onPrev,
  onNext,
  prevLabel = 'Previous',
  nextLabel = 'Next',
  onToday,
  showToday = true,
  leading,
  trailing,
  labelClassName,
  className,
}: DateNavigatorProps) {
  return (
    <div className={cn('flex items-center gap-1', className)} role="group" aria-label="Date navigation">
      {leading != null ? <span className="mr-1 flex items-center">{leading}</span> : null}
      {onToday && showToday ? (
        <Button variant="outline" size="xs" onClick={onToday} className="mr-1">
          Today
        </Button>
      ) : null}
      <IconButton
        variant="ghost"
        label={prevLabel}
        onClick={onPrev}
        icon={<DeckIcon icon={ChevronLeft} size={16} />}
      />
      <span
        aria-live="polite"
        className={cn('text-center font-display text-lead font-semibold text-foreground', labelClassName)}
      >
        {label}
      </span>
      {trailing}
      <IconButton
        variant="ghost"
        label={nextLabel}
        onClick={onNext}
        icon={<DeckIcon icon={ChevronRight} size={16} />}
      />
    </div>
  );
}
