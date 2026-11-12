'use client';

import { Input } from '@/components/ui/input';
import { cn } from '@/lib/utils';
import { formatDueDate } from '@/lib/format-date';
import type { ActionItem } from '@/types';

/**
 * Compact due-date control (WS1.a): shows the structured `dueDate` (or `dueHint` as a
 * fallback label when there's no date), and reveals a native date input to set/clear it.
 * A native `<input type="date">` keeps this dependency-free and works in the WKWebview.
 */
/** Local calendar date as `YYYY-MM-DD` (never via UTC). */
function localIso(d: Date): string {
  const pad = (n: number) => String(n).padStart(2, '0');
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
}

export function DueDateControl({
  item,
  onSetDueDate,
}: {
  item: ActionItem;
  onSetDueDate: (item: ActionItem, dueDate: string | null) => void | Promise<void>;
}) {
  const label = formatDueDate(item.dueDate) ?? item.dueHint;
  const urgent =
    item.status === 'open' && !!item.dueDate && item.dueDate.slice(0, 10) <= localIso(new Date());
  return (
    <label
      className={cn(
        'relative inline-flex cursor-pointer items-center gap-1 rounded-md px-1 py-px text-caption transition-colors hover:bg-muted focus-within:ring-2 focus-within:ring-ring',
        urgent ? 'text-brand' : label ? 'text-muted-foreground' : 'text-engrave',
      )}
      title={item.dueDate ? 'Change due date' : 'Set a due date'}
    >
      <span>{label ?? 'Due'}</span>
      {/* The native picker overlays the label; onChange sets, empty value clears. */}
      <Input
        type="date"
        aria-label="Due date"
        value={item.dueDate ?? ''}
        onChange={(e) => void onSetDueDate(item, e.target.value ? e.target.value : null)}
        className="absolute inset-0 h-auto cursor-pointer border-0 p-0 opacity-0 shadow-none"
      />
    </label>
  );
}
