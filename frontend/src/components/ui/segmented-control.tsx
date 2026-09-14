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
  className?: string;
}

/**
 * A two-or-three-way view switch drawn as underlined tabs (specs/0057 Task 2) —
 * the same 2px brand underline the meeting-details tab bar and the Settings tabs
 * use, so "switch the view" reads the same everywhere. It stays a `role="group"`
 * of `aria-pressed` buttons (not a tablist): these toggle a view, they don't
 * label panels.
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

  return (
    <div role="group" aria-label={ariaLabel} className={cn('flex items-center gap-1', className)}>
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
            tabIndex={active ? 0 : -1}
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
              'inline-flex items-center gap-1.5 border-b-2 px-3 py-1.5 text-sm font-semibold transition-colors [transition-duration:140ms] focus:outline-none focus-visible:ring-2 focus-visible:ring-ring',
              active
                ? 'border-brand text-foreground'
                : 'border-transparent text-muted-foreground hover:text-foreground'
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
