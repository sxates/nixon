'use client';

/**
 * InlineEditable — the person page's click-to-edit fields (name, title, notes). Renders as
 * text with a hover pencil; clicking swaps to an `Input` (or a `Textarea` for notes) that
 * commits on Enter/blur and reverts on Escape. Empty commits are rejected unless
 * `allowEmpty` (name is required; title and notes can be cleared).
 *
 * Not `ui/editable-text`: that primitive is controlled by its caller, always renders a
 * textarea, always commits on Enter, has no Escape-revert or placeholder, and draws its
 * display text as an `h1`. These fields need all of those, and the name already sits inside
 * PageHeader's h1.
 */

import { useCallback, useEffect, useRef, useState } from 'react';
import { DeckIcon } from '@/components/ui/deck-icon';
import { Pencil } from '@/components/ui/icons';
import { Input } from '@/components/ui/input';
import { Textarea } from '@/components/ui/textarea';
import { cn } from '@/lib/utils';

export interface InlineEditableProps {
  value: string;
  placeholder: string;
  ariaLabel: string;
  allowEmpty: boolean;
  /** Type classes for the display text. */
  textClassName?: string;
  /** Type classes for the editor, so editing doesn't jump. */
  inputClassName?: string;
  onCommit: (next: string) => void;
  /** Render a wrapping textarea (notes) instead of a single-line input. */
  multiline?: boolean;
  /** Clip the display text to one line with an ellipsis. Off for the name/notes. */
  truncate?: boolean;
}

export function InlineEditable({
  value,
  placeholder,
  ariaLabel,
  allowEmpty,
  textClassName,
  inputClassName,
  onCommit,
  multiline = false,
  truncate = true,
}: InlineEditableProps) {
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(value);
  const inputRef = useRef<HTMLInputElement>(null);
  const textareaRef = useRef<HTMLTextAreaElement>(null);

  // Keep the draft synced with the source of truth while not actively editing.
  useEffect(() => {
    if (!editing) setDraft(value);
  }, [value, editing]);

  useEffect(() => {
    if (editing) {
      const id = window.setTimeout(() => {
        const el = multiline ? textareaRef.current : inputRef.current;
        el?.focus();
        el?.select();
      }, 0);
      return () => window.clearTimeout(id);
    }
  }, [editing, multiline]);

  const commit = useCallback(() => {
    setEditing(false);
    const trimmed = draft.trim();
    if (!allowEmpty && !trimmed) {
      setDraft(value); // reject empty — revert to current value
      return;
    }
    if (trimmed !== value.trim()) onCommit(trimmed);
  }, [draft, value, allowEmpty, onCommit]);

  if (editing) {
    // Escape reverts+closes for both; single-line commits on Enter, the textarea
    // keeps Enter for newlines and commits with ⌘/Ctrl+Enter (or blur).
    const onKeyDown = (e: React.KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.preventDefault();
        setDraft(value);
        setEditing(false);
      } else if (e.key === 'Enter' && (!multiline || e.metaKey || e.ctrlKey)) {
        e.preventDefault();
        commit();
      }
    };
    if (multiline) {
      return (
        <Textarea
          ref={textareaRef}
          value={draft}
          placeholder={placeholder}
          aria-label={ariaLabel}
          rows={3}
          onChange={(e) => setDraft(e.target.value)}
          onBlur={commit}
          onKeyDown={onKeyDown}
          className={cn('resize-y bg-muted px-2 py-1.5 shadow-none', inputClassName)}
        />
      );
    }
    return (
      <Input
        ref={inputRef}
        type="text"
        value={draft}
        placeholder={placeholder}
        aria-label={ariaLabel}
        onChange={(e) => setDraft(e.target.value)}
        onBlur={commit}
        onKeyDown={onKeyDown}
        className={cn('h-auto bg-muted px-2 py-1 shadow-none', inputClassName)}
      />
    );
  }

  const display = value.trim() || placeholder;
  const isPlaceholder = !value.trim();
  return (
    // Button's centred, nowrap control chrome is the wrong shape for wrapping display text.
    <button // ui-gate: allow click-to-edit display text with a hover pencil, not a control
      type="button"
      onClick={() => setEditing(true)}
      title="Click to edit"
      aria-label={ariaLabel}
      className={cn(
        'group -ml-2 flex w-fit max-w-full gap-2 rounded-md px-2 py-1 text-left transition-colors hover:bg-muted focus:outline-none focus-visible:ring-2 focus-visible:ring-ring',
        multiline ? 'items-start' : 'items-center',
      )}
    >
      <span
        className={cn(
          truncate ? 'truncate' : 'whitespace-pre-wrap break-words',
          textClassName,
          isPlaceholder && 'text-muted-foreground',
        )}
      >
        {display}
      </span>
      <DeckIcon
        icon={Pencil}
        size={14}
        className={cn(
          'flex-shrink-0 text-muted-foreground opacity-0 transition-opacity group-hover:opacity-100 group-focus-visible:opacity-100',
          multiline && 'mt-1',
        )}
      />
    </button>
  );
}
