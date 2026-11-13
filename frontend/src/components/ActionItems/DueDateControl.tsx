'use client';

import { useState } from 'react';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { MiniCalendar } from '@/components/ui/mini-calendar';
import { Popover, PopoverContent, PopoverTrigger } from '@/components/ui/popover';
import { friendlyDue, toLocalIso } from '@/lib/due-label';
import { resolveDueHint } from '@/lib/todo-parse';
import { cn } from '@/lib/utils';
import type { ActionItem } from '@/types';

/** What the trigger shows for an item (exported for tests). */
export function dueDisplay(
  item: Pick<ActionItem, 'dueDate' | 'dueHint' | 'createdAt' | 'status'>,
  today: Date,
): { label: string | null; title: string | null; urgent: boolean } {
  if (item.dueDate) {
    const f = friendlyDue(item.dueDate, today);
    return { label: f.label, title: f.full, urgent: item.status === 'open' && f.dueSoon };
  }
  const hint = item.dueHint?.trim();
  if (!hint) return { label: null, title: null, urgent: false };
  // The hint was relative to when it was said: resolve it against the item's own local
  // creation date. Display only — nothing is written until the user picks a date.
  const created = item.createdAt ? new Date(item.createdAt) : null;
  const anchor = created && !Number.isNaN(created.getTime()) ? created : today;
  const resolved = resolveDueHint(hint, anchor);
  if (resolved) {
    const f = friendlyDue(resolved, today);
    return { label: f.label, title: `${f.full} (“${hint}”)`, urgent: false };
  }
  return { label: hint, title: hint, urgent: false };
}

/**
 * Due-date control (specs/0088): a friendly label ("Tomorrow", "Next Wednesday") that
 * opens a picker — type a date ("Friday", "next Tue", "Nov 20"), take a quick pick, or
 * click a day on the month grid.
 */
export function DueDateControl({
  item,
  onSetDueDate,
  quietWhenEmpty = false,
  size = 'caption',
  className,
}: {
  item: ActionItem;
  onSetDueDate: (item: ActionItem, dueDate: string | null) => void | Promise<void>;
  /** In a Due column: an item with no date shows "Set date" only on row hover/focus. */
  quietWhenEmpty?: boolean;
  /** `body` on the detail page, where it sits under a title-size heading. */
  size?: 'caption' | 'body';
  className?: string;
}) {
  const [open, setOpen] = useState(false);
  const [text, setText] = useState('');
  const today = new Date();
  const { label, title, urgent } = dueDisplay(item, today);

  const typed = text.trim() ? resolveDueHint(text, today) : null;
  const preview = typed ? friendlyDue(typed, today) : null;

  const apply = (iso: string | null) => {
    setOpen(false);
    if (iso !== item.dueDate) void onSetDueDate(item, iso);
  };

  const emptyLabel = quietWhenEmpty ? 'Set date' : size === 'body' ? 'Set due date' : 'Due';

  return (
    <Popover
      open={open}
      onOpenChange={(next) => {
        setOpen(next);
        if (next) setText('');
      }}
    >
      <PopoverTrigger asChild>
        <button // ui-gate: allow inline text that is itself the trigger (the date label); Button chrome would box it in the row
          type="button"
          aria-label={
            label ? `Due ${label}${title && title !== label ? `, ${title}` : ''} — change` : 'Set a due date'
          }
          title={title ?? 'Set a due date'}
          className={cn(
            'inline-flex min-w-0 cursor-pointer items-center gap-1 rounded-md px-1 py-px text-left transition-colors hover:bg-muted',
            'focus:outline-none focus-visible:ring-2 focus-visible:ring-ring data-[state=open]:bg-muted',
            size === 'body' ? 'text-sm' : 'text-caption',
            urgent ? 'text-brand' : label ? 'text-muted-foreground' : 'text-engrave',
            !label &&
              quietWhenEmpty &&
              'opacity-0 transition-opacity focus-visible:opacity-100 group-hover:opacity-100 data-[state=open]:opacity-100',
            className,
          )}
        >
          <span className="truncate">{label ?? emptyLabel}</span>
        </button>
      </PopoverTrigger>
      <PopoverContent align="start" className="w-72 p-2">
        <form
          onSubmit={(e) => {
            e.preventDefault();
            if (typed) apply(typed);
          }}
        >
          <Input
            value={text}
            onChange={(e) => setText(e.target.value)}
            placeholder="Type a date — Friday, next Tue, Nov 20"
            aria-label="Type a due date"
            className="h-8 text-sm"
            autoFocus
          />
        </form>
        <div className="mt-1 min-h-4 px-1 text-caption" aria-live="polite">
          {text.trim() ? (
            preview ? (
              <span className="text-muted-foreground">
                <span className="font-medium text-foreground">{preview.label}</span> · {preview.full}
              </span>
            ) : (
              <span className="text-muted-foreground">Not a date I understand</span>
            )
          ) : null}
        </div>
        <div className="mt-1 grid grid-cols-3 gap-1">
          <Button variant="outline" size="xs" onClick={() => apply(toLocalIso(today))}>
            Today
          </Button>
          <Button
            variant="outline"
            size="xs"
            onClick={() => apply(resolveDueHint('tomorrow', today))}
          >
            Tomorrow
          </Button>
          <Button
            variant="outline"
            size="xs"
            onClick={() => apply(resolveDueHint('next week', today))}
          >
            Next week
          </Button>
        </div>
        <MiniCalendar value={item.dueDate} onSelect={apply} today={today} className="mt-2" />
        {item.dueDate && (
          <div className="mt-1 border-t border-border pt-1">
            <Button
              variant="ghost"
              size="xs"
              className="w-full text-muted-foreground"
              onClick={() => apply(null)}
            >
              Clear date
            </Button>
          </div>
        )}
      </PopoverContent>
    </Popover>
  );
}
