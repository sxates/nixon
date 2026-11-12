'use client';

/**
 * EditableText (docs/design-system.md §Component catalog) — click-to-edit text with a
 * hover pencil. Controlled: the caller owns `isEditing` and the value. Enter commits,
 * Shift+Enter inserts a newline, blur commits. An optional `onDelete` adds a hover trash.
 *
 * Promoted from `components/EditableTitle.tsx` (its only caller is the AI summary section
 * heading); `EditableTitle` remains as an alias.
 */

import { useEffect, useRef } from 'react';

import { DeckIcon } from '@/components/ui/deck-icon';
import { Pencil, Trash2 } from '@/components/ui/icons';
import { cn } from '@/lib/utils';

export interface EditableTextProps {
  /** The current text. */
  title: string;
  isEditing: boolean;
  onStartEditing: () => void;
  onFinishEditing: () => void;
  onChange: (value: string) => void;
  onDelete?: () => void;
  /** Accessible name for the pencil button. */
  editLabel?: string;
  /** Accessible name for the trash button. */
  deleteLabel?: string;
  /** Type classes shared by the display text and the editor, so editing doesn't jump. */
  textClassName?: string;
}

export function EditableText({
  title,
  isEditing,
  onStartEditing,
  onFinishEditing,
  onChange,
  onDelete,
  editLabel = 'Edit section title',
  deleteLabel = 'Delete section',
  textClassName = 'text-2xl font-bold',
}: EditableTextProps) {
  const inputRef = useRef<HTMLTextAreaElement>(null);

  // Auto-resize the textarea to its content.
  useEffect(() => {
    if (inputRef.current && isEditing) {
      inputRef.current.style.height = 'auto';
      inputRef.current.style.height = `${inputRef.current.scrollHeight}px`;
    }
  }, [title, isEditing]);

  if (isEditing) {
    return (
      <div className="flex-1">
        <textarea
          ref={inputRef}
          value={title}
          onChange={(e) => onChange(e.target.value)}
          onBlur={onFinishEditing}
          onKeyDown={(e) => {
            // Enter commits; Shift+Enter keeps a newline.
            if (e.key === 'Enter' && !e.shiftKey) {
              e.preventDefault();
              onFinishEditing();
            }
          }}
          className={cn(
            'w-full resize-none overflow-hidden rounded-md border border-border bg-muted px-3 py-1 focus:outline-none focus-visible:ring-2 focus-visible:ring-ring',
            textClassName,
          )}
          style={{ minWidth: '300px', minHeight: '40px' }}
          autoFocus
          rows={1}
        />
      </div>
    );
  }

  const hoverButton =
    'rounded-md p-1 opacity-0 transition-opacity duration-200 hover:bg-muted focus:outline-none focus-visible:opacity-100 focus-visible:ring-2 focus-visible:ring-ring group-hover:opacity-100';

  return (
    <div className="group flex flex-1 items-center space-x-2">
      <h1
        className={cn('flex-1 cursor-pointer whitespace-pre-wrap rounded-md px-1 hover:bg-muted', textClassName)}
        onClick={onStartEditing}
      >
        {title}
      </h1>
      <div className="flex space-x-1">
        <button type="button" onClick={onStartEditing} className={hoverButton} aria-label={editLabel} title={editLabel}>
          <DeckIcon icon={Pencil} size={16} />
        </button>
        {onDelete && (
          <button
            type="button"
            onClick={onDelete}
            className={cn(hoverButton, 'text-destructive')}
            aria-label={deleteLabel}
            title={deleteLabel}
          >
            <DeckIcon icon={Trash2} size={16} />
          </button>
        )}
      </div>
    </div>
  );
}

/** @deprecated Use `EditableText`. */
export const EditableTitle = EditableText;
