"use client";

/**
 * Speaker legend + rename/merge/attendee-association UI (specs/0010, P2, Task 8).
 *
 * Since specs/0057 Plan 3 it renders on the meeting document itself, above the tabs
 * (the channel strip is meeting identity, not transcript chrome). Once a meeting has
 * been diarized it shows a color-coded list of its speakers and lets the user:
 *   - rename a speaker inline (free text)               → api_rename_speaker
 *   - name a speaker from a calendar attendee (1 click) → api_assign_speaker_to_attendee
 *   - merge two speakers (fixes over-segmentation)      → api_merge_speakers
 *
 * Colors/avatars reuse the P1 transcript color mapping (lib/speaker-colors) keyed by
 * speakerKey, so a speaker's legend dot matches its in-transcript name color. The
 * local user always shows as "You".
 *
 * Attendees come from the linked calendar event (api_get_meeting_attendees); when
 * there is none the roster is empty and we simply fall back to free-text rename —
 * no errors, no empty pick-list noise.
 */

import { useEffect, useMemo, useRef, useState } from 'react';
import { Check, ChevronDown, MoreVertical, Pencil, Sparkles, Users, X } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from '@/components/ui/popover';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog';
import { PersonFormDialog } from '@/components/People/PersonFormDialog';
import { speakerBgClass, speakerColorClass } from '@/lib/speaker-colors';
import {
  groupSeconds,
  shareOfTalk,
  talkTimeBySpeaker,
} from '@/lib/speaker-talk-time';
import { useMeetingTalkTime } from '@/hooks/meeting-details/useMeetingTalkTime';
import { ChannelStrip, type ChannelRow } from '@/components/MeetingDetails/ChannelStrip';
import { speakerChipProps, type SpeakerChipProps } from '@/components/MeetingDetails/speaker-chip-props';
import { filterPeople } from '@/lib/people-filter';
import { consolidateSpeakers } from '@/lib/speaker-consolidation';
import { cn } from '@/lib/utils';
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from '@/components/ui/tooltip';
import type {
  MeetingSpeaker,
  MeetingAttendee,
  Person,
  Transcript,
} from '@/types';
import type { UseSpeakersReturn } from '@/hooks/useSpeakers';

interface SpeakerLegendProps {
  meetingId: string | undefined;
  /** The shared speaker controller (specs/0019 WS2.1) — owned by the meeting page
   *  so this strip and the inline transcript assignment mutate one source of truth. */
  controller: UseSpeakersReturn;
  /** Re-fetch the transcript after a speaker edit (labels come from the backend join). */
  onRefetchTranscripts?: () => Promise<void>;
  /** The meeting's transcript rows — the share-of-talk source (specs/0057 §3.5). */
  transcripts?: Transcript[];
  /** Hidden while a recording is live — speakers only exist post-diarization. */
  isRecording?: boolean;
  /** specs/0061 W4 (task 3) — the speaker currently filtering the transcript
   *  (row highlight), or null/undefined when nothing is selected. */
  selectedSpeakerKey?: string | null;
  /** Click (or Enter/Space) a row: page-content owns select-vs-clear toggling
   *  and the first-line lookup + tab switch. */
  onSelectSpeaker?: (key: string | null) => void;
  className?: string;
}

/** Above this many speakers the legend offers a manual Show all/Hide toggle, so a
 *  long run (a bad diarization can produce 100+) can be folded away. */
const COLLAPSE_THRESHOLD = 6;

export function SpeakerLegend({
  meetingId,
  controller,
  onRefetchTranscripts,
  transcripts = [],
  isRecording = false,
  selectedSpeakerKey,
  onSelectSpeaker,
  className,
}: SpeakerLegendProps) {
  // Only the fields the legend itself reads; the per-chip props come from
  // `speakerChipProps(group, { controller, … })`.
  const { speakers, isLoading, refresh } = controller;

  // After editing the Person behind a speaker (specs/0017), re-fetch speakers (the
  // legend shows the resolved name) and the transcript labels.
  const handlePersonSaved = async () => {
    await refresh();
    if (onRefetchTranscripts) await onRefetchTranscripts();
  };

  // specs/0019 WS2.4 (note 9) — collapse speakers mapped to the same person into one
  // chip so two "Priya" rows don't both show. Grouping is by personId/email; the chip
  // fans corrections out across all member keys so they stay consolidated.
  const groups = useMemo(() => consolidateSpeakers(speakers), [speakers]);
  const groupPrimaries = useMemo(() => groups.map((g) => g.primary), [groups]);

  // specs/0057 §3.5 — channel rows in group order (the backend lists the local "You"
  // speaker first, so it is CH1); a consolidated group totals all its member keys. Talk
  // time comes from the WHOLE meeting, but only once there IS a speaker to attribute it
  // to — an undiarized/notes-only meeting draws no strip and must not pull every row.
  const { seconds: fullSeconds } = useMeetingTalkTime(
    speakers.length > 0 ? meetingId : undefined,
    speakers,
  );
  const rows: ChannelRow[] = useMemo(() => {
    // The whole-meeting map wins whenever we have one — including while a refetch after a
    // speaker edit is in flight (the hook keeps the last good map). The panel's paginated
    // `transcripts` is only the very-first-paint placeholder so the strip is never blank.
    const seconds = fullSeconds.size > 0 ? fullSeconds : talkTimeBySpeaker(transcripts);
    const shares = shareOfTalk(seconds);
    return groups.map((g, i) => ({
      channel: i + 1,
      index: i,
      speakerKey: g.primary.speakerKey,
      colorClass: speakerBgClass(g.primary.isLocal ? 'local' : g.primary.speakerKey),
      seconds: groupSeconds(seconds, g.keys),
      share: groupSeconds(shares, g.keys),
      // specs/0061 W4 task 3, ruling R37 — names the row's select button
      // ("Filter transcript to <name>"); the visible name cell is unaffected
      // (still `renderName`/`SpeakerChip`).
      displayName: g.primary.displayName,
    }));
  }, [groups, transcripts, fullSeconds]);

  // Default to expanded — the strip is height-capped + scrollable, so it can't push the
  // page around; the Show all/Hide toggle only appears for a long run.
  const manySpeakers = groups.length > COLLAPSE_THRESHOLD;
  const [collapsed, setCollapsed] = useState(false);

  // Nothing to show while recording, or until the meeting has speakers (diarized).
  if (isRecording || !meetingId || (speakers.length === 0 && !isLoading)) {
    return null;
  }

  const showChips = !collapsed;

  return (
    <div className={cn('flex flex-col gap-1.5', className)}>
      <div className="flex items-center gap-2">
        <span className="flex items-center gap-1 text-xs font-medium text-muted-foreground">
          <Users size={13} />
          Speakers
          {manySpeakers && (
            <span className="text-muted-foreground">({speakers.length})</span>
          )}
        </span>
        {manySpeakers && (
          <button
            type="button"
            onClick={() => setCollapsed((c) => !c)}
            aria-expanded={!collapsed}
            aria-label={
              collapsed
                ? `Show all ${speakers.length} speakers`
                : 'Hide speaker list'
            }
            className="ml-auto flex items-center gap-0.5 rounded px-1 py-0.5 text-xs text-muted-foreground hover:bg-muted hover:text-foreground"
          >
            {collapsed ? 'Show all' : 'Hide'}
            <ChevronDown
              size={13}
              className={cn(
                'transition-transform',
                collapsed ? '' : 'rotate-180',
              )}
            />
          </button>
        )}
      </div>

      {/* Channel strip (specs/0057 §3.5). Height-capped + scrollable so a long
          speaker run scrolls WITHIN the legend instead of pushing the transcript
          off-screen; chip popovers/menus use Radix portals, so the overflow
          doesn't clip them. The name cell is the unchanged SpeakerChip. */}
      {showChips && (
        <div className="max-h-[15rem] overflow-y-auto pr-1">
          <ChannelStrip
            rows={rows}
            selectedKey={selectedSpeakerKey}
            onSelect={onSelectSpeaker ? (key) => onSelectSpeaker(key) : undefined}
            renderName={(r) => (
              <SpeakerChip
                {...speakerChipProps(groups[r.index ?? r.channel - 1], {
                  allSpeakers: groupPrimaries,
                  controller,
                  onPersonSaved: handlePersonSaved,
                })}
              />
            )}
          />
        </div>
      )}
    </div>
  );
}

function SpeakerChip({
  speaker,
  memberKeys,
  allSpeakers,
  attendees,
  people,
  suggestion,
  crossMeetingSuggestion,
  onDismissCrossMeetingSuggestion,
  onRename,
  onAssignAttendee,
  onAssignPerson,
  onMerge,
  onPersonSaved,
}: SpeakerChipProps) {
  // specs/0019 WS2.4 — when this chip stands for several consolidated speakers, apply
  // every correction to all of them so the group doesn't split back apart. Sequential
  // (not Promise.all) so the per-call refresh/refetch settles once per key in order.
  const fanRename = async (_key: string, displayName: string) => {
    for (const k of memberKeys) await onRename(k, displayName);
  };
  const fanAssignAttendee = async (
    _key: string,
    attendee: { name: string; email: string },
  ) => {
    for (const k of memberKeys) await onAssignAttendee(k, attendee);
  };
  const fanAssignPerson = async (_key: string, person: Person) => {
    for (const k of memberKeys) await onAssignPerson(k, person);
  };
  // Merging a consolidated group folds every member key into the target.
  const fanMerge = async (_from: string, intoKey: string) => {
    for (const k of memberKeys) if (k !== intoKey) await onMerge(k, intoKey);
  };
  const [renameOpen, setRenameOpen] = useState(false);
  const [draftName, setDraftName] = useState(speaker.displayName);
  // WS2.7 — query for filtering the attendee/people assign lists (reset when reopened).
  const [pickQuery, setPickQuery] = useState('');
  const [editPersonOpen, setEditPersonOpen] = useState(false);
  const inputRef = useRef<HTMLInputElement>(null);

  // A speaker is "linked" to a durable Person when its email matches one (specs/0016
  // assigns via email). When so, offer an inline "Edit person" affordance (specs/0017)
  // so name/role/notes can be fixed without leaving the meeting.
  const linkedPerson = useMemo(() => {
    const email = speaker.email?.trim().toLowerCase();
    if (!email) return null;
    return people.find((p) => p.email?.trim().toLowerCase() === email) ?? null;
  }, [people, speaker.email]);

  // The 1:1 suggestion only applies to this chip if it targets this speaker key
  // and the speaker isn't already named from it.
  const matchedSuggestion =
    suggestion && suggestion.speakerKey === speaker.speakerKey ? suggestion : null;

  // Don't offer this speaker as a merge target for itself.
  const mergeTargets = useMemo(
    () => allSpeakers.filter((s) => s.speakerKey !== speaker.speakerKey),
    [allSpeakers, speaker.speakerKey],
  );

  useEffect(() => {
    if (renameOpen) {
      setDraftName(speaker.displayName);
      // Focus + select once the popover content mounts.
      requestAnimationFrame(() => inputRef.current?.select());
    }
  }, [renameOpen, speaker.displayName]);

  const submitRename = async () => {
    const trimmed = draftName.trim();
    if (trimmed && trimmed !== speaker.displayName) {
      await fanRename(speaker.speakerKey, trimmed);
    }
    setRenameOpen(false);
  };

  const pickAttendee = async (attendee: MeetingAttendee) => {
    await fanAssignAttendee(speaker.speakerKey, {
      name: attendee.name,
      email: attendee.email,
    });
    setRenameOpen(false);
  };

  const pickPerson = async (person: Person) => {
    await fanAssignPerson(speaker.speakerKey, person);
    setRenameOpen(false);
  };

  // People to offer in the pick-list. Drop any person whose email already appears
  // as a calendar attendee so the same identity isn't listed twice (attendees
  // win — they carry the meeting's own roster). Case-insensitive email match.
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

  // specs/0019 WS2.7 (note 4) — a search box over the attendee/people assign lists so
  // picking a known person stays fast as the directory grows. Separate from the rename
  // field (which is pre-filled with the speaker's current label).
  const q = pickQuery.trim().toLowerCase();
  const filteredAttendees = useMemo(
    () =>
      !q
        ? attendees
        : attendees.filter(
            (a) =>
              a.name?.toLowerCase().includes(q) || a.email?.toLowerCase().includes(q),
          ),
    [attendees, q],
  );
  const filteredPeople = useMemo(() => filterPeople(offeredPeople, pickQuery), [offeredPeople, pickQuery]);

  const acceptSuggestion = async () => {
    if (!matchedSuggestion) return;
    await fanAssignAttendee(speaker.speakerKey, {
      name: matchedSuggestion.displayName,
      email: matchedSuggestion.email,
    });
    setRenameOpen(false);
  };

  // Accept the cross-meeting "Looks like …" suggestion (specs/0016 1a). One click:
  // if it carries an email we route through the attendee-assign path (so the email
  // is recorded too); otherwise it's a plain rename. Never auto-applied on load.
  const acceptCrossMeetingSuggestion = async () => {
    if (!crossMeetingSuggestion) return;
    if (crossMeetingSuggestion.suggestedEmail) {
      await fanAssignAttendee(speaker.speakerKey, {
        name: crossMeetingSuggestion.suggestedName,
        email: crossMeetingSuggestion.suggestedEmail,
      });
    } else {
      await fanRename(speaker.speakerKey, crossMeetingSuggestion.suggestedName);
    }
    // The suggestion is now consumed; dismiss so it isn't re-offered this meeting.
    onDismissCrossMeetingSuggestion(speaker.speakerKey);
  };

  return (
    <div className="inline-flex flex-col items-start gap-1">
    {/* 0.1.0 canvas feedback: no box around the name — plain text with the pencil shown on
        hover/focus only (`group/chip`), so the strip reads as a list, not a row of buttons. */}
    <div className="group/chip inline-flex items-center gap-0.5 text-xs">
      {/* Color dot (matches the in-transcript name color). */}
      <span
        className={cn(
          'inline-block h-2.5 w-2.5 flex-shrink-0 rounded-full',
          speakerBgClass(speaker.speakerKey),
        )}
        aria-hidden
      />

      {/* Click the name to rename / pick an attendee. */}
      <Popover open={renameOpen} onOpenChange={(o) => { setRenameOpen(o); if (o) setPickQuery(''); }}>
        <PopoverTrigger asChild>
          <button
            type="button"
            className={cn(
              'flex items-center gap-1 rounded px-1 py-0.5 font-medium hover:bg-muted',
              speakerColorClass(speaker.speakerKey),
            )}
            title="Rename speaker"
          >
            {speaker.displayName}
            <Pencil
              size={10}
              className="text-muted-foreground opacity-0 transition-opacity group-hover/chip:opacity-100 group-focus-within/chip:opacity-100"
            />
          </button>
        </PopoverTrigger>
        <PopoverContent align="start" className="w-64 p-2">
          <div className="space-y-2">
            {/* Pre-filled obvious mapping, surfaced as a one-click affordance. */}
            {matchedSuggestion && (
              <button
                type="button"
                onClick={() => void acceptSuggestion()}
                className="flex w-full items-center gap-2 rounded-md border border-brand/30 bg-brand/10 px-2 py-1.5 text-left text-xs text-brand hover:bg-brand/20"
              >
                <Check size={14} className="flex-shrink-0" />
                <span className="truncate">
                  This is <strong>{matchedSuggestion.displayName}</strong>?
                </span>
              </button>
            )}

            {/* Free-text rename. */}
            <form
              onSubmit={(e) => {
                e.preventDefault();
                void submitRename();
              }}
              className="flex items-center gap-1.5"
            >
              <Input
                ref={inputRef}
                value={draftName}
                onChange={(e) => setDraftName(e.target.value)}
                placeholder="Speaker name"
                className="h-8 text-sm"
                onKeyDown={(e) => {
                  if (e.key === 'Escape') setRenameOpen(false);
                }}
              />
              <Button type="submit" size="sm" className="h-8 px-2">
                Save
              </Button>
            </form>

            {/* WS2.7 — search across the attendee + people assign lists. */}
            {(attendees.length > 0 || offeredPeople.length > 0) && (
              <Input
                value={pickQuery}
                onChange={(e) => setPickQuery(e.target.value)}
                placeholder="Search people…"
                className="h-7 text-xs"
                aria-label="Search people to assign"
              />
            )}

            {/* Attendee pick-list — only when the meeting has a linked calendar event. */}
            {attendees.length > 0 && (
              <div className="space-y-1">
                <div className="u-section-label px-1">
                  Attendees
                </div>
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

            {/* People pick-list — durable, cross-meeting identities (specs/0016 1b).
                An additional assign source alongside calendar attendees; works even
                for people who've opted out of voice modeling. */}
            {offeredPeople.length > 0 && (
              <div className="space-y-1">
                <div className="u-section-label px-1">
                  People
                </div>
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
                          <span className="ml-1 text-muted-foreground">
                            · {person.role}
                          </span>
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

            {/* Edit the durable Person behind this speaker (specs/0017) — set
                name/role/notes without leaving the meeting. Only when linked. */}
            {linkedPerson && (
              <button
                type="button"
                onClick={() => {
                  setRenameOpen(false);
                  setEditPersonOpen(true);
                }}
                className="flex w-full items-center gap-2 rounded-md border-t border-border px-2 pt-2 text-left text-xs text-muted-foreground hover:text-foreground"
              >
                <Pencil size={12} className="flex-shrink-0" />
                Edit {linkedPerson.displayName}
              </button>
            )}
          </div>
        </PopoverContent>
      </Popover>

      {/* Person editor modal for the linked person (specs/0017). onSaved re-fetches
          speakers + transcript so the resolved name updates. */}
      {linkedPerson && (
        <PersonFormDialog
          open={editPersonOpen}
          onOpenChange={setEditPersonOpen}
          person={linkedPerson}
          onSaved={onPersonSaved}
        />
      )}

      {/* Merge menu (only meaningful when there's another speaker to merge into). */}
      {mergeTargets.length > 0 && (
        <MergeMenu
          speaker={speaker}
          targets={mergeTargets}
          onMerge={fanMerge}
        />
      )}
    </div>

      {/* Cross-meeting voice-identity suggestion (specs/0016 1a). One click accepts
          (rename or attendee-assign); the X dismisses it for this meeting. Uses the
          same brand-tinted affordance as the in-popover attendee suggestion so it
          reads as the same "we think we know who this is" language. */}
      {crossMeetingSuggestion && (
        <TooltipProvider delayDuration={300}>
          <div className="inline-flex items-center gap-0.5 rounded-[3px] border border-brand/30 bg-brand/10 pl-1.5 pr-0.5 py-0.5 text-[11px] text-brand">
            <Tooltip>
              <TooltipTrigger asChild>
                <button
                  type="button"
                  onClick={() => void acceptCrossMeetingSuggestion()}
                  className="flex items-center gap-1 rounded-[3px] px-1 py-0.5 font-medium hover:bg-brand/20"
                >
                  <Sparkles size={11} className="flex-shrink-0" />
                  <span className="truncate">
                    Looks like {crossMeetingSuggestion.suggestedName}
                  </span>
                </button>
              </TooltipTrigger>
              <TooltipContent side="bottom" className="max-w-xs">
                {crossMeetingSuggestion.basis}
                <span className="mt-0.5 block text-muted-foreground">
                  Click to confirm.
                </span>
              </TooltipContent>
            </Tooltip>
            <button
              type="button"
              onClick={() =>
                onDismissCrossMeetingSuggestion(speaker.speakerKey)
              }
              className="flex-shrink-0 rounded-[3px] p-0.5 text-brand/70 hover:bg-brand/20 hover:text-brand"
              aria-label={`Dismiss suggestion for ${speaker.displayName}`}
              title="Dismiss"
            >
              <X size={11} />
            </button>
          </div>
        </TooltipProvider>
      )}
    </div>
  );
}

interface MergeMenuProps {
  speaker: MeetingSpeaker;
  targets: MeetingSpeaker[];
  onMerge: (fromKey: string, intoKey: string) => Promise<void>;
}

function MergeMenu({ speaker, targets, onMerge }: MergeMenuProps) {
  // The target we're about to merge INTO (drives the confirm dialog).
  const [pendingTarget, setPendingTarget] = useState<MeetingSpeaker | null>(null);

  return (
    <>
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <button
            type="button"
            className="rounded p-0.5 text-muted-foreground hover:bg-muted hover:text-foreground"
            title="Merge this speaker into another"
          >
            <MoreVertical size={13} />
          </button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end" className="w-52">
          <DropdownMenuLabel className="text-xs font-normal text-muted-foreground">
            Merge {speaker.displayName} into…
          </DropdownMenuLabel>
          <DropdownMenuSeparator />
          {targets.map((target) => (
            <DropdownMenuItem
              key={target.speakerKey}
              onSelect={() => setPendingTarget(target)}
              className="text-sm"
            >
              <span
                className={cn(
                  'mr-2 inline-block h-2.5 w-2.5 rounded-full',
                  speakerBgClass(target.speakerKey),
                )}
                aria-hidden
              />
              {target.displayName}
            </DropdownMenuItem>
          ))}
        </DropdownMenuContent>
      </DropdownMenu>

      <Dialog
        open={pendingTarget !== null}
        onOpenChange={(open) => {
          if (!open) setPendingTarget(null);
        }}
      >
        <DialogContent className="sm:max-w-md">
          <DialogHeader>
            <DialogTitle>Merge speakers?</DialogTitle>
            <DialogDescription>
              All of <strong>{speaker.displayName}</strong>&apos;s segments will be
              reassigned to <strong>{pendingTarget?.displayName}</strong>, and{' '}
              <strong>{speaker.displayName}</strong> will be removed. This can&apos;t be
              undone.
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="outline" onClick={() => setPendingTarget(null)}>
              Cancel
            </Button>
            <Button
              variant="destructive"
              onClick={() => {
                const target = pendingTarget;
                setPendingTarget(null);
                if (target) {
                  void onMerge(speaker.speakerKey, target.speakerKey);
                }
              }}
            >
              Merge
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </>
  );
}
