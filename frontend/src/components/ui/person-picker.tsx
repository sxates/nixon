'use client';

/**
 * PersonPicker and PersonOption (docs/design-system.md §2) — "who is this?" pickers: assign
 * an action item, name a speaker, add a participant.
 *
 * `PersonOption` is one pick-list row: the name (`font-medium`), an optional `suffix`
 * after it ("(you)", "· Product lead") and the email as a muted caption line. It replaces
 * the four hand-copied rows in AssigneePicker, InlineSpeakerAssign, SpeakerLegend and
 * ParticipantsPanel.
 *
 * `PersonPicker` is the search field over grouped option lists. It doesn't filter — each
 * caller ranks and filters its own sources — but `matchesPersonQuery` is the shared
 * name/email match. Enter in the field calls `onSubmit` (the "use the typed name" path).
 * The picker is the popover's CONTENT; the trigger and `Popover` stay the caller's.
 */

import * as React from 'react';
import { Input } from '@/components/ui/input';
import { cn } from '@/lib/utils';

export interface PersonOptionProps {
  name: React.ReactNode;
  email?: string | null;
  /** After the name: "(you)", "· role". Rendered muted. */
  suffix?: React.ReactNode;
  /** An avatar or speaker dot before the text. */
  lead?: React.ReactNode;
  onSelect: () => void;
  /** Marks the current choice. */
  selected?: boolean;
  disabled?: boolean;
  className?: string;
}

export function PersonOption({
  name,
  email,
  suffix,
  lead,
  onSelect,
  selected = false,
  disabled = false,
  className,
}: PersonOptionProps) {
  return (
    <button
      type="button"
      onClick={onSelect}
      disabled={disabled}
      aria-pressed={selected || undefined}
      className={cn(
        'flex w-full items-center gap-2 rounded-md px-2 py-1 text-left text-xs transition-colors',
        'hover:bg-muted focus:outline-none focus-visible:ring-2 focus-visible:ring-ring',
        'disabled:pointer-events-none disabled:opacity-50',
        selected && 'bg-brand-wash',
        className,
      )}
    >
      {lead != null ? <span className="flex flex-none items-center">{lead}</span> : null}
      <span className="flex min-w-0 flex-1 flex-col items-start">
        <span className="max-w-full truncate font-medium text-foreground">
          {name}
          {suffix != null ? <span className="ml-1 font-normal text-muted-foreground">{suffix}</span> : null}
        </span>
        {email ? <span className="max-w-full truncate text-caption text-muted-foreground">{email}</span> : null}
      </span>
    </button>
  );
}

/** Case-insensitive substring match on a name and (optionally) an email. Empty query matches. */
export function matchesPersonQuery(query: string, name: string, email?: string | null): boolean {
  const q = query.trim().toLowerCase();
  if (!q) return true;
  return name.toLowerCase().includes(q) || (!!email && email.toLowerCase().includes(q));
}

export interface PersonPickerGroup {
  /** Engraved group label ("Attendees", "People"). */
  label: string;
  /** Already filtered by the caller. Render `PersonOption`s. */
  options: React.ReactNode[];
  /** Shown when `options` is empty. Without it, an empty group is hidden. */
  emptyMessage?: React.ReactNode;
}

export interface PersonPickerProps {
  query: string;
  onQueryChange: (query: string) => void;
  placeholder?: string;
  /** Accessible name for the search field. Defaults to the placeholder. */
  searchLabel?: string;
  /** Enter in the field. */
  onSubmit?: () => void;
  /** A control beside the field (the "assign typed name" button). */
  inputAction?: React.ReactNode;
  /** Rows above the groups ("Me", "No assignee"). */
  before?: React.ReactNode;
  groups: PersonPickerGroup[];
  autoFocus?: boolean;
  /** Max height of each group's scroll area. Defaults to `max-h-44`. */
  listClassName?: string;
  className?: string;
}

export function PersonPicker({
  query,
  onQueryChange,
  placeholder = 'Search people…',
  searchLabel,
  onSubmit,
  inputAction,
  before,
  groups,
  autoFocus = true,
  listClassName,
  className,
}: PersonPickerProps) {
  const visibleGroups = groups.filter((g) => g.options.length > 0 || g.emptyMessage != null);
  return (
    <div className={cn('space-y-2', className)}>
      <form
        role="search"
        onSubmit={(e) => {
          e.preventDefault();
          onSubmit?.();
        }}
        className="flex items-center gap-1.5"
      >
        <Input
          value={query}
          onChange={(e) => onQueryChange(e.target.value)}
          placeholder={placeholder}
          aria-label={searchLabel ?? placeholder}
          className="h-8 text-sm"
          autoFocus={autoFocus}
        />
        {inputAction}
      </form>
      {before != null ? <div className="space-y-0.5">{before}</div> : null}
      {visibleGroups.map((group, i) => (
        <div
          key={group.label}
          role="group"
          aria-label={group.label}
          className={cn('space-y-1', (i > 0 || before != null) && 'border-t border-border pt-2')}
        >
          <div className="u-section-label px-1" aria-hidden="true">
            {group.label}
          </div>
          {group.options.length === 0 ? (
            <div className="px-2 py-1 text-caption text-muted-foreground">{group.emptyMessage}</div>
          ) : (
            <div className={cn('max-h-44 space-y-0.5 overflow-y-auto', listClassName)}>{group.options}</div>
          )}
        </div>
      ))}
    </div>
  );
}
