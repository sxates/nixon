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
import { ChevronDown } from '@/components/ui/icons';
import { DeckIcon } from '@/components/ui/deck-icon';
import { PersonOption, PersonPicker, matchesPersonQuery } from '@/components/ui/person-picker';
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from '@/components/ui/popover';
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

  const q = query.trim();
  const filteredAttendees = useMemo(
    () => attendees.filter((a) => matchesPersonQuery(query, a.name ?? '', a.email)),
    [attendees, query],
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
          'u-speaker whitespace-nowrap',
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
        <button // ui-gate: allow the transcript speaker name IS the trigger: typed caps in the speaker colour inside a virtualized row, not Button chrome
          type="button"
          className={cn(
            'u-speaker group -ml-1 flex items-center gap-0.5 whitespace-nowrap rounded-md px-1 py-0 hover:bg-muted focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring',
            speakerColorClass(speakerKey),
          )}
          title="Assign this speaker to a person"
        >
          {speakerName}
          <DeckIcon
            icon={ChevronDown}
            size={12}
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
            <PersonPicker
              query={query}
              onQueryChange={setQuery}
              placeholder="Assign to a person…"
              searchLabel="Search people to assign"
              groups={[
                ...(attendees.length > 0
                  ? [
                      {
                        label: 'Attendees',
                        options: filteredAttendees.map((attendee) => (
                          <PersonOption
                            key={attendee.email || attendee.name}
                            name={attendee.name}
                            email={attendee.email}
                            suffix={attendee.isCurrentUser ? '(you)' : undefined}
                            onSelect={() => void pickAttendee(attendee)}
                          />
                        )),
                      },
                    ]
                  : []),
                ...(offeredPeople.length > 0
                  ? [
                      {
                        label: 'People',
                        options: filteredPeople.map((person) => (
                          <PersonOption
                            key={person.id}
                            name={person.displayName}
                            email={person.email}
                            suffix={person.role ? `· ${person.role}` : undefined}
                            onSelect={() => void pickPerson(person)}
                          />
                        )),
                      },
                    ]
                  : []),
              ]}
            />
          )}

          {q && filteredAttendees.length === 0 && filteredPeople.length === 0 && (
            <div className="px-2 py-1 text-xs text-muted-foreground">No matches</div>
          )}
        </div>
      </PopoverContent>
    </Popover>
  );
}
