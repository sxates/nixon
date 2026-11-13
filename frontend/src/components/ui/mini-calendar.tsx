'use client';

/**
 * MiniCalendar (docs/design-system.md §2) — a compact month grid for picking one date,
 * inside a popover (the to-do due-date picker).
 *
 * Weeks start on Monday and the grid always draws six weeks so the popover never changes
 * height between months. Today is amber text; the selected date is the lit amber key;
 * days from the neighbouring months are engraved. The grid is one Tab stop: arrow keys
 * move a day / a week (crossing into the next month as needed), Enter or Space picks.
 *
 * Dates are LOCAL `YYYY-MM-DD` strings in and out — never via UTC.
 */

import * as React from 'react';
import { DeckIcon } from '@/components/ui/deck-icon';
import { IconButton } from '@/components/ui/icon-button';
import { ChevronLeft, ChevronRight } from '@/components/ui/icons';
import { fullDate, parseLocalDate, toLocalIso } from '@/lib/due-label';
import { cn } from '@/lib/utils';

const MONTHS = [
  'January',
  'February',
  'March',
  'April',
  'May',
  'June',
  'July',
  'August',
  'September',
  'October',
  'November',
  'December',
];
const WEEKDAY_INITIALS = ['M', 'T', 'W', 'T', 'F', 'S', 'S'];
const WEEKDAY_NAMES = ['Monday', 'Tuesday', 'Wednesday', 'Thursday', 'Friday', 'Saturday', 'Sunday'];

export interface MiniCalendarProps {
  /** The selected date (`YYYY-MM-DD`), or null. */
  value?: string | null;
  /** Called with the picked date (`YYYY-MM-DD`). */
  onSelect: (iso: string) => void;
  /** Defaults to now; injectable for tests. */
  today?: Date;
  className?: string;
}

const firstOfMonth = (d: Date) => new Date(d.getFullYear(), d.getMonth(), 1);
const addDays = (d: Date, n: number) => new Date(d.getFullYear(), d.getMonth(), d.getDate() + n);
const sameMonth = (a: Date, b: Date) =>
  a.getFullYear() === b.getFullYear() && a.getMonth() === b.getMonth();

/** The 42 days shown for `month`: from the Monday on/before the 1st. */
function gridDays(month: Date): Date[] {
  const first = firstOfMonth(month);
  const start = addDays(first, -((first.getDay() + 6) % 7));
  return Array.from({ length: 42 }, (_, i) => addDays(start, i));
}

export function MiniCalendar({ value, onSelect, today: todayProp, className }: MiniCalendarProps) {
  const today = todayProp ?? new Date();
  const selected = value ? parseLocalDate(value) : null;
  const todayIso = toLocalIso(today);
  const selectedIso = selected ? toLocalIso(selected) : null;

  const [view, setView] = React.useState(() => firstOfMonth(selected ?? today));
  // The roving-focus day: the selected date, else today, else the 1st of the view.
  const [focused, setFocused] = React.useState<Date>(() => selected ?? today);
  const focusAfterMove = React.useRef(false);
  const gridRef = React.useRef<HTMLDivElement>(null);

  const days = gridDays(view);
  const focusedIso = toLocalIso(sameMonth(focused, view) ? focused : view);

  React.useEffect(() => {
    if (!focusAfterMove.current) return;
    focusAfterMove.current = false;
    gridRef.current?.querySelector<HTMLButtonElement>(`[data-date="${focusedIso}"]`)?.focus();
  }, [focusedIso]);

  const stepMonth = (n: number) => {
    const next = new Date(view.getFullYear(), view.getMonth() + n, 1);
    setView(next);
    // Keep the day of month where it exists, so arrowing back and forth is stable.
    const last = new Date(next.getFullYear(), next.getMonth() + 1, 0).getDate();
    setFocused(new Date(next.getFullYear(), next.getMonth(), Math.min(focused.getDate(), last)));
  };

  const onKeyDown = (e: React.KeyboardEvent) => {
    const delta = { ArrowLeft: -1, ArrowRight: 1, ArrowUp: -7, ArrowDown: 7 }[e.key];
    if (delta === undefined) return;
    e.preventDefault();
    const base = sameMonth(focused, view) ? focused : view;
    const next = addDays(base, delta);
    focusAfterMove.current = true;
    setFocused(next);
    if (!sameMonth(next, view)) setView(firstOfMonth(next));
  };

  const monthLabel = `${MONTHS[view.getMonth()]} ${view.getFullYear()}`;

  return (
    <div className={cn('select-none', className)}>
      <div className="mb-1 flex items-center justify-between gap-1">
        <IconButton
          variant="ghost"
          label="Previous month"
          icon={<DeckIcon icon={ChevronLeft} />}
          onClick={() => stepMonth(-1)}
        />
        <div className="text-sm font-semibold text-foreground" aria-live="polite">
          {monthLabel}
        </div>
        <IconButton
          variant="ghost"
          label="Next month"
          icon={<DeckIcon icon={ChevronRight} />}
          onClick={() => stepMonth(1)}
        />
      </div>
      <div ref={gridRef} role="grid" aria-label={monthLabel} onKeyDown={onKeyDown}>
        <div role="row" className="grid grid-cols-7">
          {WEEKDAY_INITIALS.map((initial, i) => (
            <div
              key={i}
              role="columnheader"
              aria-label={WEEKDAY_NAMES[i]}
              className="flex h-6 items-center justify-center text-caption text-engrave"
            >
              {initial}
            </div>
          ))}
        </div>
        {Array.from({ length: 6 }, (_, w) => (
          <div key={w} role="row" className="grid grid-cols-7">
            {days.slice(w * 7, w * 7 + 7).map((d) => {
              const iso = toLocalIso(d);
              const isSelected = iso === selectedIso;
              const isToday = iso === todayIso;
              const outside = !sameMonth(d, view);
              return (
                <div key={iso} role="gridcell" aria-selected={isSelected} className="flex justify-center">
                  <button
                    type="button"
                    data-date={iso}
                    tabIndex={iso === focusedIso ? 0 : -1}
                    aria-label={fullDate(d)}
                    aria-current={isToday ? 'date' : undefined}
                    onClick={() => onSelect(iso)}
                    onFocus={() => setFocused(d)}
                    className={cn(
                      'flex h-8 w-8 items-center justify-center rounded-md text-xs tabular-nums transition-colors',
                      'focus:outline-none focus-visible:ring-2 focus-visible:ring-ring',
                      isSelected
                        ? 'bg-brand font-semibold text-brand-foreground'
                        : cn(
                            'hover:bg-muted',
                            isToday
                              ? 'font-semibold text-brand ring-1 ring-inset ring-brand-edge'
                              : outside
                                ? 'text-engrave'
                                : 'text-foreground',
                          ),
                    )}
                  >
                    {d.getDate()}
                  </button>
                </div>
              );
            })}
          </div>
        ))}
      </div>
    </div>
  );
}
