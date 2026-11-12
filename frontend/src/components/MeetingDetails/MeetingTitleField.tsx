'use client';

/**
 * The click-to-rename meeting title, shared by the meeting page and the recorder so both
 * headers are the same `PageHeader` (docs/design-system.md §Component catalog). It renders
 * INSIDE `PageHeader`'s `<h1 class="u-page-title">`, so it carries no heading of its own and
 * inherits the 22px display type; the editor uses the same type so editing doesn't jump.
 *
 * Controlled: the caller owns the draft, the editing flag and the commit/cancel semantics
 * (the two callers persist differently). Enter commits, Escape cancels, blur commits.
 *
 * Not `ui/EditableText`: that renders its own `<h1>` (it would nest inside PageHeader's),
 * edits in a textarea, and has no Escape-to-revert or caller-owned input ref, which the
 * recorder's title hook focuses itself.
 */

import type { RefObject } from 'react';

import { Button } from '@/components/ui/button';
import { DeckIcon } from '@/components/ui/deck-icon';
import { Pencil } from '@/components/ui/icons';
import { Input } from '@/components/ui/input';
import { cn } from '@/lib/utils';

export interface MeetingTitleFieldProps {
  /** The title to show. Empty shows `placeholder`, muted. */
  title: string;
  /** Shown (muted) when there is no title yet. */
  placeholder: string;
  /** False renders static text: nothing to rename yet. */
  editable?: boolean;
  isEditing: boolean;
  draft: string;
  onDraftChange: (value: string) => void;
  onStartEditing: () => void;
  onCommit: () => void;
  onCancel: () => void;
  inputRef?: RefObject<HTMLInputElement>;
}

export function MeetingTitleField({
  title,
  placeholder,
  editable = true,
  isEditing,
  draft,
  onDraftChange,
  onStartEditing,
  onCommit,
  onCancel,
  inputRef,
}: MeetingTitleFieldProps) {
  const text = title.trim();
  const shown = text || placeholder;
  const tone = text ? 'text-foreground' : 'text-muted-foreground';

  if (editable && isEditing) {
    return (
      <Input
        ref={inputRef}
        type="text"
        value={draft}
        placeholder={placeholder}
        aria-label="Meeting title"
        onChange={(e) => onDraftChange(e.target.value)}
        onBlur={onCommit}
        onKeyDown={(e) => {
          if (e.key === 'Enter') {
            e.preventDefault();
            onCommit();
          } else if (e.key === 'Escape') {
            e.preventDefault();
            onCancel();
          }
        }}
        // -ml-2 + px-2 keeps the text where the display button draws it.
        className="u-page-title -ml-2 h-auto bg-muted px-2 py-0.5 text-title shadow-none md:text-title"
      />
    );
  }

  if (!editable) {
    return <span className={cn('block truncate', tone)}>{shown}</span>;
  }

  return (
    <Button
      type="button"
      variant="ghost"
      onClick={onStartEditing}
      title="Click to rename"
      className={cn(
        'group -ml-2 h-auto max-w-full justify-start px-2 py-0.5 text-left font-display text-title font-semibold',
        tone,
      )}
    >
      <span className="truncate">{shown}</span>
      <DeckIcon
        icon={Pencil}
        size={16}
        className="flex-shrink-0 text-muted-foreground opacity-0 transition-opacity group-hover:opacity-100 group-focus-visible:opacity-100"
      />
    </Button>
  );
}
