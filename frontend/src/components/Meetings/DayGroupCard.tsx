'use client';

/**
 * One day's card on the Meetings page — the List view's day buckets and the Week view's
 * seven days share it, so the two read as the same object (design-system audit B,
 * 2026-11-11: the List drew engraved caps "3 RECORDINGS" while Week drew a sentence-case
 * "3 meetings" header with different padding).
 *
 * A `Panel` with no padding of its own; the header is a `ListRow` (weekday as the row
 * title, the date beside it as row meta, the count trailing), and the day's rows sit under
 * a soft `border-rule` hairline. Today's card wears the brand edge and a brand weekday.
 * With `onOpen` the header is a button (Week drills into that day's agenda).
 */

import type { ReactNode } from 'react';
import { ListRow } from '@/components/ui/list-row';
import { Panel } from '@/components/ui/panel';
import { cn } from '@/lib/utils';

/** "Free", "1 meeting", "3 meetings" — the one count wording for a day. */
export function dayCountLabel(count: number): string {
  if (count === 0) return 'Free';
  return `${count} meeting${count === 1 ? '' : 's'}`;
}

export function DayGroupCard({
  label,
  dateLabel,
  count,
  isToday = false,
  onOpen,
  children,
}: {
  /** "Today", "Monday". */
  label: string;
  /** "Nov 10" (empty for the Undated bucket). */
  dateLabel?: string;
  count: number;
  isToday?: boolean;
  /** Makes the header a button. */
  onOpen?: () => void;
  /** The day's rows (`ListRow ruled`). Nothing renders under the header when absent. */
  children?: ReactNode;
}) {
  return (
    <Panel
      data-day-card=""
      data-today={isToday || undefined}
      className={cn('overflow-hidden p-0', isToday && 'border-brand-edge')}
    >
      <ListRow
        onClick={onOpen}
        className={cn('rounded-none', !onOpen && 'hover:bg-transparent')}
        title={
          <>
            <span className={isToday ? 'text-brand' : undefined}>{label}</span>
            {dateLabel ? <span className="u-row-meta ml-2 font-normal">{dateLabel}</span> : null}
          </>
        }
        trailing={<span className="u-row-meta tabular-nums">{dayCountLabel(count)}</span>}
      />
      {children ? (
        <div className="border-t border-rule [&>*:last-child]:border-b-0">{children}</div>
      ) : null}
    </Panel>
  );
}
