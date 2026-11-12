'use client';

/**
 * AssigneePicker (specs/0034) — the assignee chip on an action-item row. Opens a
 * popover mirroring the Participants add pop-list (`components/Participants/`):
 * a "Me" shortcut, the candidate pick-list (the meeting roster on the meeting page,
 * the People directory in the task hub) live-filtered by the text field, a free-text
 * path for names not in the roster (stored as `assignee_raw`), and a clear action.
 */

import { useEffect, useMemo, useState } from 'react';
import { UserPlus } from '@/components/ui/icons';
import { Button } from '@/components/ui/button';
import { DeckIcon } from '@/components/ui/deck-icon';
import { IconButton } from '@/components/ui/icon-button';
import { PersonOption, PersonPicker } from '@/components/ui/person-picker';
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from '@/components/ui/popover';
import { cn } from '@/lib/utils';
import { filterPeople } from '@/lib/people-filter';
import type { AssigneeSelection } from '@/lib/action-items';

export type { AssigneeSelection };

/** A pickable assignee: a roster participant or a People-directory person. */
export interface AssigneeCandidate {
  personId: string;
  displayName: string;
  email?: string | null;
}

interface AssigneePickerProps {
  /** Current display label ("Me" / a name), or null when unassigned. */
  label: string | null;
  candidates: AssigneeCandidate[];
  onSelect: (selection: AssigneeSelection) => void | Promise<void>;
  /** Chip text while unassigned (default "Assign"; the Inbox row says "Assign…"). */
  emptyLabel?: string;
  className?: string;
}

export function AssigneePicker({
  label,
  candidates,
  onSelect,
  emptyLabel = 'Assign',
  className,
}: AssigneePickerProps) {
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState('');

  // Reset the draft whenever the popover reopens.
  useEffect(() => {
    if (open) setQuery('');
  }, [open]);

  // Shared people-search filter (name/role/email) — same matching as every picker.
  const filtered = useMemo(() => filterPeople(candidates, query), [candidates, query]);

  const pick = (selection: AssigneeSelection) => {
    setOpen(false);
    void onSelect(selection);
  };

  // Free-text submit: an exact name match resolves to that person; otherwise the
  // typed name is kept verbatim as the unresolved assignee.
  const submitFreeText = () => {
    const name = query.trim();
    if (!name) return;
    const exact = candidates.find(
      (c) => c.displayName.trim().toLowerCase() === name.toLowerCase(),
    );
    pick(exact ? { kind: 'person', personId: exact.personId } : { kind: 'raw', name });
  };

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <Button
          type="button"
          variant={label ? 'outline' : 'dashed'}
          size="xs"
          title={label ? `Assigned to ${label} — change` : 'Assign to someone'}
          className={cn('max-w-[180px]', className)}
        >
          <span className="truncate">{label ?? emptyLabel}</span>
        </Button>
      </PopoverTrigger>
      <PopoverContent align="start" className="w-64 p-2">
        <PersonPicker
          query={query}
          onQueryChange={setQuery}
          placeholder="Search or type a name…"
          onSubmit={submitFreeText}
          inputAction={
            <IconButton
              type="submit"
              size="md"
              label="Assign the typed name"
              disabled={!query.trim()}
              icon={<DeckIcon icon={UserPlus} />}
            />
          }
          before={
            <>
              <PersonOption name="Me" onSelect={() => pick({ kind: 'me' })} />
              {label && (
                <PersonOption
                  name={<span className="font-normal text-muted-foreground">No assignee</span>}
                  onSelect={() => pick({ kind: 'clear' })}
                />
              )}
            </>
          }
          listClassName="max-h-48"
          groups={
            candidates.length > 0
              ? [
                  {
                    label: 'People',
                    options: filtered.map((candidate) => (
                      <PersonOption
                        key={candidate.personId}
                        name={candidate.displayName}
                        email={candidate.email}
                        onSelect={() => pick({ kind: 'person', personId: candidate.personId })}
                      />
                    )),
                    emptyMessage: (
                      <>
                        No people match “{query.trim()}”. Press enter to assign the name as
                        typed.
                      </>
                    ),
                  },
                ]
              : []
          }
        />
      </PopoverContent>
    </Popover>
  );
}
