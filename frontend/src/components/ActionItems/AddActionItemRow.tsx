'use client';

/**
 * AddActionItemRow (specs/0034) — the "Add item" row shared by the per-meeting
 * section and the task hub. A plain input + button; Enter or the button submits
 * (the parent owns the IPC call). The draft clears only after `onAdd` succeeds —
 * a failed create (return `false` or throw) keeps the typed text for retry.
 */

import { useState } from 'react';
import { Plus } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { cn } from '@/lib/utils';

interface AddActionItemRowProps {
  placeholder?: string;
  /** Create handler. Return `false` (or throw) to signal failure and keep the draft. */
  onAdd: (description: string) => boolean | void | Promise<boolean | void>;
  className?: string;
}

export function AddActionItemRow({
  placeholder = 'Add an action item…',
  onAdd,
  className,
}: AddActionItemRowProps) {
  const [draft, setDraft] = useState('');
  const [isSubmitting, setIsSubmitting] = useState(false);

  const submit = async () => {
    const description = draft.trim();
    if (!description || isSubmitting) return;
    setIsSubmitting(true);
    try {
      const ok = await onAdd(description);
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
      <Input
        value={draft}
        onChange={(e) => setDraft(e.target.value)}
        placeholder={placeholder}
        aria-label={placeholder}
        className="h-8 text-sm"
      />
      <Button
        type="submit"
        size="sm"
        variant="outline"
        className="h-8 flex-shrink-0 gap-1 px-2.5"
        disabled={!draft.trim() || isSubmitting}
      >
        <Plus size={13} />
        Add
      </Button>
    </form>
  );
}
