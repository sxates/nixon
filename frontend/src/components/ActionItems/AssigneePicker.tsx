'use client';

/**
 * AssigneePicker (specs/0034) — the assignee chip on an action-item row. Opens a
 * popover mirroring the Participants add pop-list (`components/Participants/`):
 * a "Me" shortcut, the candidate pick-list (the meeting roster on the meeting page,
 * the People directory in the task hub) live-filtered by the text field, a free-text
 * path for names not in the roster (stored as `assignee_raw`), and a clear action.
 */

import { useEffect, useMemo, useState } from 'react';
import { UserCheck, UserPlus, UserRound, X } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
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
  className?: string;
}

export function AssigneePicker({
  label,
  candidates,
  onSelect,
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
        <button
          type="button"
          title={label ? `Assigned to ${label} — change` : 'Assign to someone'}
          className={cn(
            'inline-flex max-w-[180px] items-center gap-1 rounded-[3px] border px-2 py-0.5 text-[11px] font-medium transition-colors',
            label
              ? 'border-border bg-card text-foreground hover:border-brand/50 hover:text-brand'
              : 'border-dashed border-border text-muted-foreground hover:border-brand/50 hover:text-brand',
            className,
          )}
        >
          <UserRound size={11} className="flex-shrink-0" />
          <span className="truncate">{label ?? 'Assign'}</span>
        </button>
      </PopoverTrigger>
      <PopoverContent align="start" className="w-64 p-2">
        <div className="space-y-2">
          <form
            onSubmit={(e) => {
              e.preventDefault();
              submitFreeText();
            }}
            className="flex items-center gap-1.5"
          >
            <Input
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              placeholder="Search or type a name…"
              className="h-8 text-sm"
              autoFocus
            />
            <Button
              type="submit"
              size="sm"
              variant="outline"
              className="h-8 gap-1 px-2"
              disabled={!query.trim()}
              title="Assign the typed name"
            >
              <UserPlus size={13} />
            </Button>
          </form>

          <div className="space-y-0.5">
            <button
              type="button"
              onClick={() => pick({ kind: 'me' })}
              className="flex w-full items-center gap-2 rounded px-2 py-1 text-left text-xs font-medium text-foreground hover:bg-muted"
            >
              <UserCheck size={13} className="text-muted-foreground" />
              Me
            </button>
            {label && (
              <button
                type="button"
                onClick={() => pick({ kind: 'clear' })}
                className="flex w-full items-center gap-2 rounded px-2 py-1 text-left text-xs text-muted-foreground hover:bg-muted hover:text-foreground"
              >
                <X size={13} />
                No assignee
              </button>
            )}
          </div>

          {candidates.length > 0 && (
            <div className="space-y-1 border-t border-border pt-2">
              <div className="u-section-label px-1">People</div>
              {filtered.length === 0 ? (
                <div className="px-2 py-1 text-[11px] text-muted-foreground">
                  No people match “{query.trim()}”. Press enter to assign the name as
                  typed.
                </div>
              ) : (
                <div className="max-h-48 space-y-0.5 overflow-y-auto">
                  {filtered.map((candidate) => (
                    <button
                      key={candidate.personId}
                      type="button"
                      onClick={() => pick({ kind: 'person', personId: candidate.personId })}
                      className="flex w-full flex-col items-start rounded px-2 py-1 text-left text-xs hover:bg-muted"
                    >
                      <span className="font-medium text-foreground">
                        {candidate.displayName}
                      </span>
                      {candidate.email && (
                        <span className="truncate text-[11px] text-muted-foreground">
                          {candidate.email}
                        </span>
                      )}
                    </button>
                  ))}
                </div>
              )}
            </div>
          )}
        </div>
      </PopoverContent>
    </Popover>
  );
}
