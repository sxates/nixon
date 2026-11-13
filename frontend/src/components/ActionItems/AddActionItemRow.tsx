'use client';

/**
 * AddActionItemRow (specs/0034) — the "Add item" row shared by the per-meeting
 * section and the task hub. A plain input + button; Enter or the button submits
 * (the parent owns the IPC call). The draft clears only after `onAdd` succeeds —
 * a failed create (return `false` or throw) keeps the typed text for retry.
 *
 * A trailing due phrase — `Send the deck (Friday)` — is split off on submit with
 * `parseTodoText` (specs/0088) and handed over as `dueDate`, an ISO local date. `@Name`
 * opens a People picker; picked names go out as `@[Name](person:<id>)` tokens.
 */

import { useState } from 'react';
import { Plus } from '@/components/ui/icons';
import { Button } from '@/components/ui/button';
import { cn } from '@/lib/utils';
import { parseTodoText } from '@/lib/todo-parse';
import { MentionInput } from './MentionInput';

interface AddActionItemRowProps {
  placeholder?: string;
  /** Create handler. Return `false` (or throw) to signal failure and keep the draft. */
  onAdd: (description: string, dueDate?: string) => boolean | void | Promise<boolean | void>;
  className?: string;
  /** Accessible name for the field (⌘T focuses the To-do page's by this label). */
  ariaLabel?: string;
}

export function AddActionItemRow({
  placeholder = 'Add an action item…',
  onAdd,
  className,
  ariaLabel,
}: AddActionItemRowProps) {
  const [draft, setDraft] = useState('');
  const [isSubmitting, setIsSubmitting] = useState(false);

  const submit = async (raw: string = draft) => {
    const text = raw.trim();
    if (!text || isSubmitting) return;
    const { title, dueDate } = parseTodoText(text, new Date());
    const description = title || text;
    setIsSubmitting(true);
    try {
      const ok = await (dueDate && title ? onAdd(description, dueDate) : onAdd(description));
      if (ok !== false) setDraft('');
    } catch {
      // The parent surfaces the error; keep the draft so the text isn't lost.
    } finally {
      setIsSubmitting(false);
    }
  };

  return (
    <form
      onSubmit={(e) => {
        e.preventDefault();
        void submit();
      }}
      className={cn('flex items-center gap-1.5', className)}
    >
      <MentionInput
        value={draft}
        onChange={setDraft}
        onSubmit={(v) => void submit(v)}
        placeholder={placeholder}
        ariaLabel={ariaLabel}
        className="h-8 text-sm"
      />
      <Button
        type="submit"
        size="sm"
        variant="outline"
        className="h-8 flex-shrink-0 gap-1 px-2.5"
        disabled={!draft.trim() || isSubmitting}
      >
        <Plus />
        Add
      </Button>
    </form>
  );
}
