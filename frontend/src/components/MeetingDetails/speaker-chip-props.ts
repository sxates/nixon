/**
 * Props for the legend's `SpeakerChip`, and the pure builder that assembles them from a
 * consolidated speaker group + the shared speakers controller (specs/0057 §3.5).
 *
 * Extracted from `SpeakerLegend.tsx` when the chip cloud became the channel strip: the
 * chip is now rendered from a `renderName` callback rather than an inline `.map`, and the
 * legend file is at its size-ratchet cap. Behaviour is unchanged — this is the same props
 * object the removed loop built, key for key.
 */

import type { ConsolidatedSpeaker } from '@/lib/speaker-consolidation';
import type { UseSpeakersReturn } from '@/hooks/useSpeakers';
import {
  ownerActionContext,
  type OwnerActionContext,
} from '@/components/MeetingDetails/SpeakerOwnerAction';
import type {
  MeetingSpeaker,
  MeetingAttendee,
  AttendeeSuggestion,
  SpeakerSuggestion,
  Person,
} from '@/types';

export interface SpeakerChipProps {
  speaker: MeetingSpeaker;
  /** All `speaker_key`s this chip represents (specs/0019 WS2.4). Usually just the
   *  primary; >1 when several speakers were consolidated to one person. Corrections
   *  fan out across all of them so they stay consolidated. */
  memberKeys: string[];
  allSpeakers: MeetingSpeaker[];
  attendees: MeetingAttendee[];
  /** Durable People (specs/0016 1b) offered alongside calendar attendees. */
  people: Person[];
  suggestion: AttendeeSuggestion | null;
  /** Cross-meeting voice-identity suggestion for this speaker, if any (specs/0016 1a). */
  crossMeetingSuggestion: SpeakerSuggestion | null;
  onDismissCrossMeetingSuggestion: (speakerKey: string) => void;
  onRename: (speakerKey: string, displayName: string) => Promise<void>;
  onAssignAttendee: (
    speakerKey: string,
    attendee: { name: string; email: string },
  ) => Promise<void>;
  onAssignPerson: (speakerKey: string, person: Person) => Promise<void>;
  onMerge: (fromKey: string, intoKey: string) => Promise<void>;
  /** Re-fetch speakers + transcript after editing the Person behind a speaker. */
  onPersonSaved: () => void | Promise<void>;
  /** specs/0078 — "This is me" / "This isn't me" wiring. */
  owner: OwnerActionContext;
  /** The group's cached directory photo (`controller.speakerPhotos`, first member key that
   *  has one); null => the colour dot. */
  photoDataUri: string | null;
}

/** Build one chip's props — the exact shape the legend's removed chip-cloud loop passed. */
export function speakerChipProps(
  group: ConsolidatedSpeaker,
  ctx: {
    allSpeakers: MeetingSpeaker[];
    controller: UseSpeakersReturn;
    onPersonSaved: () => void | Promise<void>;
  },
): SpeakerChipProps {
  const c = ctx.controller;
  return {
    speaker: group.primary,
    memberKeys: group.keys,
    allSpeakers: ctx.allSpeakers,
    attendees: c.attendees,
    people: c.people,
    suggestion: c.suggestion,
    crossMeetingSuggestion:
      group.keys.map((k) => c.crossMeetingSuggestions.get(k)).find(Boolean) ??
      null,
    onDismissCrossMeetingSuggestion: c.dismissSuggestion,
    onRename: c.renameSpeaker,
    onAssignAttendee: c.assignAttendee,
    onAssignPerson: c.assignPerson,
    onMerge: c.mergeSpeakers,
    onPersonSaved: ctx.onPersonSaved,
    owner: ownerActionContext(c),
    photoDataUri:
      [group.primary.speakerKey, ...group.keys]
        .map((k) => c.speakerPhotos.get(k))
        .find(Boolean) ?? null,
  };
}
