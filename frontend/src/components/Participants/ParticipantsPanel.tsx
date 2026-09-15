'use client';

/**
 * ParticipantsPanel — the editable participant roster for a meeting (specs/0017, Phase C).
 *
 * Distinct from the SpeakerLegend (who actually *spoke*): this lists the *invited/known*
 * people for a meeting, seeded from the linked calendar event (auto-People) and
 * hand-curatable. Used in BOTH contexts:
 *   - the meeting detail page (`variant="card"`), visible after recording, and
 *   - while recording (`variant="compact"`, rendered inside a popover off the record
 *     screen) for the in-progress meeting.
 *
 * Affordances (mirrors the SpeakerLegend assign pop-list for consistency):
 *   - Add: pick an existing Person (api_list_people_ranked, minus those already rostered) OR
 *     type a name/email → api_add_meeting_participant.
 *   - Remove: inline × per row → api_remove_meeting_participant (optimistic, revert on
 *     error). Removing un-rosters; it never deletes the Person or any speaker assignment.
 *   - Edit: click a participant → the shared PersonFormDialog (set role/name/notes
 *     without leaving the meeting); onSaved re-fetches the roster.
 *
 * Best-effort: a failed fetch shows an empty/error state, never throws to the page.
 */

import { useCallback, useEffect, useMemo, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { toast } from 'sonner';
import { AtSign, MoreHorizontal, Plus, UserCheck, UserPlus, Users, X } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu';
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from '@/components/ui/popover';
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from '@/components/ui/tooltip';
import { PersonFormDialog } from '@/components/People/PersonFormDialog';
import { speakerBgClass } from '@/lib/speaker-colors';
import { splitForDisplay } from '@/lib/list-overflow';
import { filterPeople } from '@/lib/people-filter';
import { MEETING_PARTICIPANTS_CHANGED_EVENT } from '@/lib/participants-events';
import { cn } from '@/lib/utils';
import type {
  MeetingAttendee,
  MeetingAttendeesResponse,
  MeetingParticipant,
  MeetingSpeaker,
  Person,
} from '@/types';

interface ParticipantsPanelProps {
  meetingId: string | undefined;
  /**
   * - `card` (default): full bordered section for the meeting detail page.
   * - `compact`: borderless, denser layout for the in-recording popover.
   */
  variant?: 'card' | 'compact';
  /**
   * Optional speakers for this meeting. When provided, a participant whose email
   * matches a speaker's email is badged "spoke" (read-only cross-reference — never
   * an auto-binding). Omit while recording (speakers aren't resolved yet).
   */
  speakers?: MeetingSpeaker[];
  className?: string;
}

/** Initials for the avatar dot, e.g. "Priya Sharma" → "PS". */
function initials(name: string): string {
  const parts = name.trim().split(/\s+/).filter(Boolean);
  if (parts.length === 0) return '?';
  if (parts.length === 1) return parts[0]!.charAt(0).toUpperCase();
  return (parts[0]!.charAt(0) + parts[parts.length - 1]!.charAt(0)).toUpperCase();
}

export function ParticipantsPanel({
  meetingId,
  variant = 'card',
  speakers,
  className,
}: ParticipantsPanelProps) {
  const [participants, setParticipants] = useState<MeetingParticipant[]>([]);
  const [people, setPeople] = useState<Person[]>([]);
  // Distribution-list attendees for this meeting (specs/0038 WS3): group addresses the
  // backend couldn't expand to individuals. Not people — rendered as a labeled floor,
  // never rostered/speaker-bound. Sourced from the linked calendar event, best-effort.
  const [distributionLists, setDistributionLists] = useState<MeetingAttendee[]>([]);
  const [isLoading, setIsLoading] = useState(true);
  const [error, setError] = useState(false);
  const [addOpen, setAddOpen] = useState(false);
  // specs/0019 WS3.1 — collapse long rosters to a cap with a "+N more" expander.
  const [expanded, setExpanded] = useState(false);
  const PARTICIPANT_CAP = 10;
  // Person currently being edited via the shared modal; null = closed.
  const [personToEdit, setPersonToEdit] = useState<Person | null>(null);

  const isCompact = variant === 'compact';

  const loadParticipants = useCallback(async () => {
    if (!meetingId) {
      setParticipants([]);
      setIsLoading(false);
      return;
    }
    try {
      const result = await invoke<MeetingParticipant[]>('api_get_meeting_participants', {
        meetingId,
      });
      setParticipants(Array.isArray(result) ? result : []);
      setError(false);
    } catch (err) {
      // Best-effort: never crash the page. Show an empty/error state instead.
      console.error('Failed to load participants:', err);
      setError(true);
    } finally {
      setIsLoading(false);
    }
  }, [meetingId]);

  // People list backs the "add existing" pick-list. Ranked (specs/0038 WS5.a) so
  // starred + frequent collaborators surface first. Reloaded after an edit so a
  // renamed person shows up correctly.
  const loadPeople = useCallback(async () => {
    try {
      const result = await invoke<Person[]>('api_list_people_ranked');
      setPeople(Array.isArray(result) ? result : []);
    } catch (err) {
      console.error('Failed to load people:', err);
    }
  }, []);

  // Distribution lists come from the linked calendar event's attendees (specs/0038 WS3).
  // Best-effort: a meeting with no calendar link (or a failed read) simply has none.
  const loadDistributionLists = useCallback(async () => {
    if (!meetingId) {
      setDistributionLists([]);
      return;
    }
    try {
      const roster = await invoke<MeetingAttendeesResponse>('api_get_meeting_attendees', {
        meetingId,
      });
      const dls = (roster?.attendees ?? []).filter((a) => a.isDistributionList);
      setDistributionLists(dls);
    } catch (err) {
      console.error('Failed to load distribution lists:', err);
      setDistributionLists([]);
    }
  }, [meetingId]);

  useEffect(() => {
    void loadParticipants();
  }, [loadParticipants]);

  useEffect(() => {
    void loadPeople();
  }, [loadPeople]);

  // specs/0038 WS6.c — identifying a transcript speaker also rosters that person as a
  // participant on the backend. That mutation happens in a decoupled sibling (the speaker
  // legend / inline assign, via useSpeakers), which fires a window event; re-fetch the
  // roster here so the newly-identified person appears without a manual reload.
  useEffect(() => {
    if (!meetingId) return;
    const onChanged = (event: Event) => {
      const detail = (event as CustomEvent<{ meetingId?: string }>).detail;
      if (!detail?.meetingId || detail.meetingId === meetingId) {
        void loadParticipants();
      }
    };
    window.addEventListener(MEETING_PARTICIPANTS_CHANGED_EVENT, onChanged);
    return () => window.removeEventListener(MEETING_PARTICIPANTS_CHANGED_EVENT, onChanged);
  }, [meetingId, loadParticipants]);

  useEffect(() => {
    void loadDistributionLists();
  }, [loadDistributionLists]);

  // Emails of speakers who actually spoke — drives the read-only "spoke" badge.
  const spokeEmails = useMemo(
    () =>
      new Set(
        (speakers ?? [])
          .map((s) => s.email?.trim().toLowerCase())
          .filter((e): e is string => !!e),
      ),
    [speakers],
  );

  // People not already on the roster, offered in the add pick-list.
  const rosterIds = useMemo(
    () => new Set(participants.map((p) => p.personId)),
    [participants],
  );
  const offerablePeople = useMemo(
    () => people.filter((p) => !rosterIds.has(p.id)),
    [people, rosterIds],
  );

  const handleAddExisting = useCallback(
    async (person: Person) => {
      if (!meetingId) return;
      setAddOpen(false);
      try {
        const added = await invoke<MeetingParticipant>('api_add_meeting_participant', {
          meetingId,
          personId: person.id,
        });
        setParticipants((prev) =>
          prev.some((p) => p.personId === added.personId) ? prev : [...prev, added],
        );
        toast.success(`Added ${added.displayName}`);
      } catch (err) {
        console.error('Failed to add participant:', err);
        toast.error('Could not add participant', {
          description: err instanceof Error ? err.message : String(err),
        });
      }
    },
    [meetingId],
  );

  const handleAddNew = useCallback(
    async (displayName: string, email: string) => {
      if (!meetingId) return;
      const name = displayName.trim();
      const mail = email.trim();
      if (!name && !mail) {
        toast.error('Enter a name or email');
        return;
      }
      setAddOpen(false);
      try {
        const added = await invoke<MeetingParticipant>('api_add_meeting_participant', {
          meetingId,
          displayName: name || null,
          email: mail || null,
        });
        setParticipants((prev) =>
          prev.some((p) => p.personId === added.personId) ? prev : [...prev, added],
        );
        // Refresh the people directory so a freshly-created person is offerable elsewhere.
        void loadPeople();
        toast.success(`Added ${added.displayName}`);
      } catch (err) {
        console.error('Failed to add participant:', err);
        toast.error('Could not add participant', {
          description: err instanceof Error ? err.message : String(err),
        });
      }
    },
    [meetingId, loadPeople],
  );

  const handleRemove = useCallback(
    async (participant: MeetingParticipant) => {
      if (!meetingId) return;
      // Optimistic remove; revert on failure.
      const snapshot = participants;
      setParticipants((prev) => prev.filter((p) => p.personId !== participant.personId));
      try {
        await invoke('api_remove_meeting_participant', {
          meetingId,
          personId: participant.personId,
        });
        toast.success(`Removed ${participant.displayName}`, {
          description: "They're still saved in People.",
        });
      } catch (err) {
        console.error('Failed to remove participant:', err);
        setParticipants(snapshot); // revert
        toast.error('Could not remove participant', {
          description: err instanceof Error ? err.message : String(err),
        });
      }
    },
    [meetingId, participants],
  );

  // "This is me" (specs/0018): fold this person into the owner ("You"). The person
  // leaves the roster app-wide, so we drop the chip optimistically. Distinct from
  // remove (×): that un-rosters one meeting; this is an identity statement.
  const handleClaimAsMe = useCallback(
    async (participant: MeetingParticipant) => {
      if (!meetingId) return;
      const label = participant.email?.trim() || participant.displayName?.trim() || 'this person';
      const snapshot = participants;
      setParticipants((prev) => prev.filter((p) => p.personId !== participant.personId));
      try {
        await invoke('api_claim_participant_as_me', {
          meetingId,
          personId: participant.personId,
        });
        toast.success(`Claimed ${label} as you`);
        // The claim may have added an owner email / merged people; refresh the directory.
        void loadPeople();
      } catch (err) {
        console.error('Failed to claim participant as me:', err);
        setParticipants(snapshot); // revert
        toast.error('Could not claim as you', {
          description: err instanceof Error ? err.message : String(err),
        });
      }
    },
    [meetingId, participants, loadPeople],
  );

  // Open the shared Person modal for a rostered participant. We hydrate a full
  // Person from the directory when we have it; otherwise build a minimal stand-in
  // from the roster row so the modal still edits the right id.
  const handleEdit = useCallback(
    (participant: MeetingParticipant) => {
      const existing = people.find((p) => p.id === participant.personId);
      setPersonToEdit(
        existing ?? {
          id: participant.personId,
          email: participant.email,
          displayName: participant.displayName,
          role: participant.role,
          notes: null,
          voiceprintOptOut: false,
          starred: false,
          createdAt: '',
          updatedAt: '',
        },
      );
    },
    [people],
  );

  // After an edit, re-fetch both the roster (names/roles) and the directory.
  const handleSaved = useCallback(async () => {
    await Promise.all([loadParticipants(), loadPeople()]);
  }, [loadParticipants, loadPeople]);

  if (!meetingId) return null;

  const addControl = (
    <AddParticipantPopover
      open={addOpen}
      onOpenChange={setAddOpen}
      offerablePeople={offerablePeople}
      onAddExisting={handleAddExisting}
      onAddNew={handleAddNew}
      compact={isCompact}
    />
  );

  const header = (
    <div className="flex items-center gap-2">
      <span className="flex items-center gap-1.5 text-xs font-medium text-muted-foreground">
        <Users size={13} />
        Participants
        {participants.length > 0 && (
          <span className="text-muted-foreground">({participants.length})</span>
        )}
      </span>
      <div className="ml-auto">{addControl}</div>
    </div>
  );

  const list =
    participants.length === 0 ? (
      <p className="text-xs text-muted-foreground">
        {error
          ? 'Could not load participants.'
          : isLoading
            ? 'Loading…'
            : 'No participants yet. Add the people in this meeting.'}
      </p>
    ) : (
      <div
        className={cn(
          'flex flex-wrap items-center gap-x-5 gap-y-2',
          isCompact && 'max-h-56 overflow-y-auto pr-1',
        )}
      >
        {splitForDisplay(participants, PARTICIPANT_CAP, expanded).shown.map((participant) => {
          const spoke =
            !!participant.email && spokeEmails.has(participant.email.trim().toLowerCase());
          return (
            <ParticipantChip
              key={participant.personId}
              participant={participant}
              spoke={spoke}
              onEdit={() => handleEdit(participant)}
              onRemove={() => void handleRemove(participant)}
              onClaimAsMe={() => void handleClaimAsMe(participant)}
            />
          );
        })}
        {(() => {
          const { hidden } = splitForDisplay(participants, PARTICIPANT_CAP, expanded);
          if (hidden > 0) {
            return (
              <button
                type="button"
                onClick={() => setExpanded(true)}
                className="rounded-[3px] border border-dashed border-border px-2.5 py-1 text-xs font-medium text-muted-foreground hover:text-foreground"
              >
                +{hidden} more
              </button>
            );
          }
          if (expanded && participants.length > PARTICIPANT_CAP) {
            return (
              <button
                type="button"
                onClick={() => setExpanded(false)}
                className="rounded-[3px] border border-dashed border-border px-2.5 py-1 text-xs font-medium text-muted-foreground hover:text-foreground"
              >
                Show less
              </button>
            );
          }
          return null;
        })()}
      </div>
    );

  // Distribution-list floor (specs/0038 WS3 / 0027 Phase 3): a DL is not a person, so it
  // renders as a labeled row (never an avatar/speaker chip). "Add members" is the manual
  // escape hatch — it reuses the exact same add pop-list (setAddOpen), so the user pulls
  // real people out of the group by hand when the backend couldn't auto-expand it.
  const dlSection =
    distributionLists.length > 0 ? (
      <div className="flex flex-col gap-1.5">
        {distributionLists.map((dl) => {
          const dlLabel = dl.name?.trim() || dl.email?.trim() || 'Distribution list';
          return (
            <div
              key={dl.email || dlLabel}
              className="flex items-center gap-2 rounded-lg border border-dashed border-border bg-muted/40 px-2.5 py-1.5 text-xs"
            >
              <span
                className="flex h-5 w-5 flex-shrink-0 items-center justify-center rounded-full bg-muted text-muted-foreground"
                aria-hidden="true"
              >
                <AtSign size={12} />
              </span>
              <div className="min-w-0 flex-1">
                <div className="truncate font-medium text-foreground">{dlLabel}</div>
                <div className="text-[11px] text-muted-foreground">
                  Distribution list — members added as they RSVP
                </div>
              </div>
              <button
                type="button"
                onClick={() => setAddOpen(true)}
                className="flex-shrink-0 rounded-[3px] border border-dashed border-border px-2 py-0.5 text-[11px] font-medium text-muted-foreground hover:border-brand/50 hover:text-brand"
              >
                Add members
              </button>
            </div>
          );
        })}
      </div>
    ) : null;

  return (
    <div
      className={cn(
        isCompact
          ? 'flex flex-col gap-2'
          : 'flex flex-col gap-2 rounded-[3px] border border-border bg-card px-4 py-3 shadow-[0_1px_2px_hsl(34_12%_12%/0.03)]',
        className,
      )}
    >
      {header}
      {list}
      {dlSection}

      <PersonFormDialog
        open={personToEdit !== null}
        onOpenChange={(open) => {
          if (!open) setPersonToEdit(null);
        }}
        person={personToEdit}
        onSaved={handleSaved}
      />
    </div>
  );
}

interface ParticipantChipProps {
  participant: MeetingParticipant;
  spoke: boolean;
  onEdit: () => void;
  onRemove: () => void;
  onClaimAsMe: () => void;
}

function ParticipantChip({
  participant,
  spoke,
  onEdit,
  onRemove,
  onClaimAsMe,
}: ParticipantChipProps) {
  const name = participant.displayName?.trim() || 'Unnamed';
  const colorKey = participant.email || name;
  // Render the cached directory photo (specs/0038 WS3) in place of initials when
  // present + loadable; a broken image (onError) or absent photo falls back to the
  // colored initials chip — mirroring AvatarStack for visual consistency.
  const [photoFailed, setPhotoFailed] = useState(false);
  const showPhoto = !!participant.photoDataUri && !photoFailed;
  return (
    <TooltipProvider delayDuration={400}>
      {/* 0.1.0 canvas feedback: unboxed — avatar + name as plain text; the secondary actions
          (more / remove) appear on hover or keyboard focus only. */}
      <div className="group inline-flex items-center gap-1.5 text-xs">
        {showPhoto ? (
          // A self-contained base64 `data:` URI — no image server in the Tauri shell,
          // so a plain <img> (not next/image) is correct here.
          // eslint-disable-next-line @next/next/no-img-element
          <img
            src={participant.photoDataUri as string}
            alt=""
            aria-hidden="true"
            onError={() => setPhotoFailed(true)}
            className="h-5 w-5 flex-shrink-0 rounded-full object-cover"
          />
        ) : (
          <span
            className={cn(
              'flex h-5 w-5 flex-shrink-0 items-center justify-center rounded-full text-[9px] font-semibold',
              // `bg-muted` is the null-key fallback and is a light chip — its ink is the
              // foreground, not the background.
              speakerBgClass(colorKey) === 'bg-muted' ? 'text-foreground' : 'text-background',
              speakerBgClass(colorKey),
            )}
            aria-hidden="true"
          >
            {initials(name)}
          </span>
        )}

        {/* Click the body to edit the person (name / role / notes). */}
        <Tooltip>
          <TooltipTrigger asChild>
            <button
              type="button"
              onClick={onEdit}
              className="flex items-center gap-1 rounded px-0.5 py-0.5 font-medium text-foreground hover:text-brand"
              title={`Edit ${name}`}
            >
              {/* specs/0019 WS3.1 (note 2): the call roster doesn't need titles —
                  name + "spoke" badge only. Role still shows in the edit dialog. */}
              <span className="max-w-[160px] truncate">{name}</span>
              {spoke && (
                <span className="rounded-[3px] bg-success/10 px-1.5 py-px text-[10px] font-medium text-success">
                  spoke
                </span>
              )}
            </button>
          </TooltipTrigger>
          <TooltipContent side="bottom">Edit this person</TooltipContent>
        </Tooltip>

        {/* Secondary actions: "This is me" lives in an overflow menu so the primary
            affordances stay edit (body) + remove (×). */}
        <DropdownMenu>
          <Tooltip>
            <TooltipTrigger asChild>
              <DropdownMenuTrigger asChild>
                <button
                  type="button"
                  aria-label={`More actions for ${name}`}
                  className="flex-shrink-0 rounded-[3px] p-0.5 text-muted-foreground opacity-0 transition-opacity hover:bg-muted hover:text-foreground focus-visible:opacity-100 group-hover:opacity-100 group-focus-within:opacity-100 data-[state=open]:opacity-100"
                >
                  <MoreHorizontal size={12} />
                </button>
              </DropdownMenuTrigger>
            </TooltipTrigger>
            <TooltipContent side="bottom">More actions</TooltipContent>
          </Tooltip>
          <DropdownMenuContent align="end">
            <DropdownMenuItem onSelect={onClaimAsMe}>
              <UserCheck size={14} className="mr-2" />
              This is me
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>

        <Tooltip>
          <TooltipTrigger asChild>
            <button
              type="button"
              onClick={onRemove}
              aria-label={`Remove ${name}`}
              className="flex-shrink-0 rounded-[3px] p-0.5 text-muted-foreground opacity-0 transition-opacity hover:bg-muted hover:text-foreground focus-visible:opacity-100 group-hover:opacity-100 group-focus-within:opacity-100"
            >
              <X size={12} />
            </button>
          </TooltipTrigger>
          <TooltipContent side="bottom">
            Remove from this meeting (stays in People)
          </TooltipContent>
        </Tooltip>
      </div>
    </TooltipProvider>
  );
}

interface AddParticipantPopoverProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  offerablePeople: Person[];
  onAddExisting: (person: Person) => void | Promise<void>;
  onAddNew: (displayName: string, email: string) => void | Promise<void>;
  compact: boolean;
}

/** The "Add participant" pop-list — mirrors the SpeakerLegend assign affordance:
 *  a People pick-list plus a free-text create-new path (name and/or email). */
function AddParticipantPopover({
  open,
  onOpenChange,
  offerablePeople,
  onAddExisting,
  onAddNew,
  compact,
}: AddParticipantPopoverProps) {
  const [name, setName] = useState('');
  const [email, setEmail] = useState('');

  // Reset the draft whenever the popover reopens.
  useEffect(() => {
    if (open) {
      setName('');
      setEmail('');
    }
  }, [open]);

  // specs/0019 WS2.7 (note 4) — the name field doubles as a live filter over the People
  // pick-list, so the roster stays usable as the directory grows: type to narrow to a
  // match (click it), or keep typing a new name and create.
  const filteredPeople = useMemo(
    () => filterPeople(offerablePeople, name),
    [offerablePeople, name],
  );

  return (
    <Popover open={open} onOpenChange={onOpenChange}>
      <PopoverTrigger asChild>
        <button
          type="button"
          className={cn(
            'inline-flex items-center gap-1 rounded-[3px] border border-dashed border-border px-2 py-0.5 text-xs font-medium text-muted-foreground hover:border-brand/50 hover:text-brand',
          )}
          title="Add participant"
        >
          <Plus size={12} />
          {compact ? 'Add' : 'Add participant'}
        </button>
      </PopoverTrigger>
      <PopoverContent align="end" className="w-72 p-2">
        <div className="space-y-2">
          {/* Create-new path: name and/or email. */}
          <form
            onSubmit={(e) => {
              e.preventDefault();
              void onAddNew(name, email);
            }}
            className="space-y-1.5"
          >
            <Input
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder="Search or add by name…"
              className="h-8 text-sm"
              autoFocus
            />
            <div className="flex items-center gap-1.5">
              <Input
                value={email}
                onChange={(e) => setEmail(e.target.value)}
                placeholder="Email (optional)"
                type="email"
                className="h-8 text-sm"
              />
              <Button type="submit" size="sm" className="h-8 gap-1 px-2">
                <UserPlus size={13} />
                Add
              </Button>
            </div>
          </form>

          {/* Existing People pick-list (those not already on the roster), filtered by the
              name field above (WS2.7). */}
          {offerablePeople.length > 0 && (
            <div className="space-y-1 border-t border-border pt-2">
              <div className="u-section-label px-1">People</div>
              {filteredPeople.length === 0 ? (
                <div className="px-2 py-1 text-[11px] text-muted-foreground">
                  No people match “{name.trim()}”. Add a new one above.
                </div>
              ) : (
              <div className="max-h-48 space-y-0.5 overflow-y-auto">
                {filteredPeople.map((person) => (
                  <button
                    key={person.id}
                    type="button"
                    onClick={() => void onAddExisting(person)}
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
              )}
            </div>
          )}
        </div>
      </PopoverContent>
    </Popover>
  );
}
