'use client';

/**
 * specs/0019 WS2.1 (note 3) — inline speaker→person assignment in the transcript.
 *
 * Renders a transcript line's speaker name as a button; clicking it opens a compact
 * picker (search + calendar attendees + durable People) that assigns the *whole*
 * speaker (by `speakerKey`) to the chosen identity via the shared `useSpeakers`
 * controller — the same backend path as the SpeakerLegend chip. So a user can name a
 * speaker right where they read them, instead of hunting for the separate legend.
 *
 * This is assignment only (pick a known person/attendee). Free-text rename and merge
 * still live in the legend; keeping the inline surface to "this is <known person>"
 * matches the note ("change a speaker to a participant in-line").
 */

import { useMemo, useState } from 'react';
import { Check, ChevronDown } from 'lucide-react';
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from '@/components/ui/popover';
import { Input } from '@/components/ui/input';
import { filterPeople } from '@/lib/people-filter';
import { cn } from '@/lib/utils';
import { speakerColorClass } from '@/lib/speaker-colors';
import type { MeetingAttendee, Person } from '@/types';
import {
  ownerActionFor,
  SpeakerOwnerAction,
  type OwnerActionContext,
} from './SpeakerOwnerAction';

export interface InlineSpeakerAssignProps {
  speakerKey: string;
  speakerName: string;
  attendees: MeetingAttendee[];
  people: Person[];
  onAssignAttendee: (
    speakerKey: string,
    attendee: { name: string; email: string },
  ) => Promise<void>;
  onAssignPerson: (speakerKey: string, person: Person) => Promise<void>;
  /** specs/0078 — "This is me" / "This isn't me"; absent = not offered. */
  owner?: OwnerActionContext;
}

export function InlineSpeakerAssign({
  speakerKey,
  speakerName,
  attendees,
  people,
  onAssignAttendee,
  onAssignPerson,
  owner,
}: InlineSpeakerAssignProps) {
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState('');

  // Drop people already present as a calendar attendee (matched by email) so the
  // same identity isn't offered twice — attendees win (the meeting's own roster).
  const attendeeEmails = useMemo(
    () =>
      new Set(
        attendees
          .map((a) => a.email?.trim().toLowerCase())
          .filter((e): e is string => !!e),
      ),
    [attendees],
  );
  const offeredPeople = useMemo(
    () =>
      people.filter((p) => {
        const email = p.email?.trim().toLowerCase();
        return !(email && attendeeEmails.has(email));
      }),
    [people, attendeeEmails],
  );

  const q = query.trim().toLowerCase();
  const filteredAttendees = useMemo(
    () =>
      !q
        ? attendees
        : attendees.filter(
            (a) =>
              a.name?.toLowerCase().includes(q) ||
              a.email?.toLowerCase().includes(q),
          ),
    [attendees, q],
  );
  const filteredPeople = useMemo(
    () => filterPeople(offeredPeople, query),
    [offeredPeople, query],
  );

  const pickAttendee = async (attendee: MeetingAttendee) => {
    setOpen(false);
    await onAssignAttendee(speakerKey, { name: attendee.name, email: attendee.email });
  };
  const pickPerson = async (person: Person) => {
    setOpen(false);
    await onAssignPerson(speakerKey, person);
  };

  // No identities to pick from (and no owner action) → just render the name (still
  // styled), no affordance.
  const hasPickList = attendees.length > 0 || offeredPeople.length > 0;
  const hasOwnerAction =
    !!owner && ownerActionFor(speakerKey, speakerKey === 'local', owner) !== null;
  const hasOptions = hasPickList || hasOwnerAction;
  if (!hasOptions) {
    return (
      <span
        className={cn(
          'u-typed font-bold uppercase text-[12px] tracking-[0.02em] whitespace-nowrap',
          speakerColorClass(speakerKey),
        )}
      >
        {speakerName}
      </span>
    );
  }

  return (
    <Popover
      open={open}
      onOpenChange={(o) => {
        setOpen(o);
        if (o) setQuery('');
      }}
    >
      <PopoverTrigger asChild>
        <button
          type="button"
          className={cn(
            'group flex items-center gap-0.5 rounded px-1 -ml-1 py-0 u-typed font-bold uppercase text-[12px] tracking-[0.02em] whitespace-nowrap hover:bg-muted',
            speakerColorClass(speakerKey),
          )}
          title="Assign this speaker to a person"
        >
          {speakerName}
          <ChevronDown
            size={11}
            className="text-muted-foreground opacity-0 transition-opacity group-hover:opacity-100"
          />
        </button>
      </PopoverTrigger>
      <PopoverContent align="start" className="w-64 p-2">
        <div className="space-y-2">
          <SpeakerOwnerAction
            speakerKey={speakerKey}
            isLocal={speakerKey === 'local'}
            owner={owner}
            onBeforeAction={() => setOpen(false)}
          />

          {hasPickList && (
            <Input
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              placeholder="Assign to a person…"
              className="h-8 text-sm"
              aria-label="Search people to assign"
              autoFocus
            />
          )}

          {attendees.length > 0 && (
            <div className="space-y-1">
              <div className="u-section-label px-1">Attendees</div>
              <div className="max-h-44 space-y-0.5 overflow-y-auto">
                {filteredAttendees.map((attendee) => (
                  <button
                    key={attendee.email || attendee.name}
                    type="button"
                    onClick={() => void pickAttendee(attendee)}
                    className="flex w-full flex-col items-start rounded px-2 py-1 text-left text-xs hover:bg-muted"
                  >
                    <span className="font-medium text-foreground">
                      {attendee.name}
                      {attendee.isCurrentUser && (
                        <span className="ml-1 text-muted-foreground">(you)</span>
                      )}
                    </span>
                    {attendee.email && (
                      <span className="truncate text-[11px] text-muted-foreground">
                        {attendee.email}
                      </span>
                    )}
                  </button>
                ))}
              </div>
            </div>
          )}

          {offeredPeople.length > 0 && (
            <div className="space-y-1">
              <div className="u-section-label px-1">People</div>
              <div className="max-h-44 space-y-0.5 overflow-y-auto">
                {filteredPeople.map((person) => (
                  <button
                    key={person.id}
                    type="button"
                    onClick={() => void pickPerson(person)}
                    className="flex w-full flex-col items-start rounded px-2 py-1 text-left text-xs hover:bg-muted"
                  >
                    <span className="font-medium text-foreground">
                      {person.displayName}
                      {person.role && (
                        <span className="ml-1 text-muted-foreground">· {person.role}</span>
                      )}
                    </span>
                    {person.email && (
                      <span className="truncate text-[11px] text-muted-foreground">
                        {person.email}
                      </span>
                    )}
                  </button>
                ))}
              </div>
            </div>
          )}

          {q && filteredAttendees.length === 0 && filteredPeople.length === 0 && (
            <div className="flex items-center gap-1.5 px-2 py-1 text-xs text-muted-foreground">
              <Check size={12} className="opacity-0" />
              No matches
            </div>
          )}
        </div>
      </PopoverContent>
    </Popover>
  );
}
