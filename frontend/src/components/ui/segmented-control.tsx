'use client';

import * as React from 'react';

import { cn } from '@/lib/utils';

export interface SegmentedOption {
  value: string;
  label: string;
  icon?: React.ReactNode;
}

export interface SegmentedControlProps {
  options: SegmentedOption[];
  value: string;
  onChange: (value: string) => void;
  'aria-label': string;
  /**
   * `default` (28px) for a page-level filter; `compact` (24px) inside a panel or toolbar;
   * `lg` (32px) when it sits in a row of default-height (32px) buttons, like Ask's scope bar.
   */
  size?: 'default' | 'compact' | 'lg';
  className?: string;
}

/**
 * A two-to-four-way FILTER drawn as a boxed key group: a recessed `bg-muted` track with the
 * active option raised as a `bg-card` cell (docs/design-system.md §Component catalog).
 *
 * **Tabs switch the view; a boxed SegmentedControl filters it; never two underline strips
 * in a row.** It used to draw underline tabs like `Tabs`, so Action items showed two
 * underline strips side by side and Ask's 7d/30d scope read as navigation. A control that
 * changes WHICH view is on screen (Meetings Agenda/Week/Month/List) is `Tabs`.
 *
 * Sizes: `default` is 28px (the `xs` control height), `compact` 24px for a panel or toolbar,
 * `lg` 32px to match default-height buttons beside it.
 * It stays a `role="group"` of `aria-pressed` buttons (not a tablist): these filter what a
 * view shows, they don't label panels.
 *
 * Keyboard: the group is ONE Tab stop (roving tabIndex — only the active option
 * is tabbable), so Arrow/Home/End must move DOM focus as well as the selection.
 * Selecting without focusing would strand the user on a button that just became
 * `tabIndex={-1}`, with no way back to the other options.
 */
export function SegmentedControl({
  options,
  value,
  onChange,
  'aria-label': ariaLabel,
  size = 'default',
  className,
}: SegmentedControlProps) {
  const buttonRefs = React.useRef<Array<HTMLButtonElement | null>>([]);

  /** Focus `index` and select it. Index is always derived from the CURRENT value. */
  const select = (index: number) => {
    const target = options[index];
    if (!target) return;
    buttonRefs.current[index]?.focus();
    if (target.value !== value) onChange(target.value);
  };

  const move = (delta: number) => {
    const current = options.findIndex((o) => o.value === value);
    const from = current === -1 ? 0 : current;
    select((from + delta + options.length) % options.length);
  };

  // No option active (a value outside the set): the first option holds the Tab stop, or
  // the group would drop out of the Tab order entirely.
  const noneActive = !options.some((o) => o.value === value);

  return (
    <div
      role="group"
      aria-label={ariaLabel}
      data-size={size}
      className={cn(
        'inline-flex flex-none items-stretch gap-0.5 rounded-md border border-border bg-muted p-0.5',
        size === 'compact' ? 'h-6' : size === 'lg' ? 'h-8' : 'h-7',
        className,
      )}
    >
      {options.map((option, index) => {
        const active = option.value === value;
        return (
          <button
            key={option.value}
            ref={(el) => {
              buttonRefs.current[index] = el;
            }}
            type="button"
            aria-pressed={active}
            // Roving tabindex: only the active option is in the Tab order.
            tabIndex={active || (noneActive && index === 0) ? 0 : -1}
            onClick={() => {
              if (!active) onChange(option.value);
            }}
            onKeyDown={(e) => {
              if (e.key === 'ArrowRight') {
                e.preventDefault();
                move(1);
              } else if (e.key === 'ArrowLeft') {
                e.preventDefault();
                move(-1);
              } else if (e.key === 'Home') {
                e.preventDefault();
                select(0);
              } else if (e.key === 'End') {
                e.preventDefault();
                select(options.length - 1);
              }
            }}
            className={cn(
              'inline-flex items-center gap-1.5 rounded-sm font-semibold transition-colors [transition-duration:140ms] focus:outline-none focus-visible:ring-2 focus-visible:ring-ring',
              size === 'compact' ? 'px-2 text-caption' : 'px-3 text-xs',
              active
                ? 'bg-card text-foreground shadow-sm'
                : 'text-muted-foreground hover:text-foreground'
            )}
          >
            {option.icon}
            {option.label}
          </button>
        );
      })}
    </div>
  );
}
