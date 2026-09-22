'use client';

/**
 * The collapsed one-line form of a meeting's participant roster: a few faces and a
 * sentence.
 *
 * Owner feedback 2026-09-21 — "the participants box gets really tall with several rows of
 * participants. For meetings with 4+ participants, lets create a new collapsed version that
 * summarizes number of participants with a few avatars, similar to All Meetings view, with
 * an affordance to expand/collapse the whole list."
 *
 * The roster grid caps at ten and wraps into an auto-fit grid roughly four wide (specs/0064
 * W4), so five people already cost two rows and ten cost three — all of it above the
 * document tabs, pushing the summary and transcript down the page on exactly the meetings
 * worth reading. Below four people the grid is one row and there is nothing to collapse.
 *
 * Lives in its own file because `ParticipantsPanel` is already 600-odd lines against an
 * 800-line cap.
 */

import { AvatarStack } from '@/components/AvatarStack';
import type { AgendaAttendee } from '@/lib/day-agenda';
import type { MeetingParticipant } from '@/types';

/** At this many participants the card starts collapsed. Three fit on one row. */
export const PARTICIPANT_COLLAPSE_THRESHOLD = 4;

/** How many names the summary sentence spells out before "and N others". */
const NAMED = 3;

/**
 * Reuse `AvatarStack` — the same faces All Meetings and Today draw — rather than a fourth
 * avatar implementation. `isCurrentUser: false` for every row because the participant
 * roster already excludes the device owner (they are not a `people` row).
 */
function asAttendees(participants: MeetingParticipant[]): AgendaAttendee[] {
  return participants.map((p) => ({
    name: p.displayName,
    email: p.email,
    isCurrentUser: false,
    photoDataUri: p.photoDataUri ?? null,
  }));
}

/**
 * "Maya, Tomas and Inès", "Maya, Tomas, Inès and 4 others".
 *
 * Spelled out with "and" rather than the list views' terser "Maya, +2": this is a sentence
 * in a card with room for one, not a cell in a dense row.
 */
export function participantSummaryText(participants: MeetingParticipant[]): string {
  const names = participants.map((p) => p.displayName.trim()).filter(Boolean);
  if (names.length === 0) return '';
  if (names.length === 1) return names[0];
  if (names.length <= NAMED) {
    return `${names.slice(0, -1).join(', ')} and ${names[names.length - 1]}`;
  }
  const remaining = names.length - NAMED;
  return `${names.slice(0, NAMED).join(', ')} and ${remaining} other${remaining === 1 ? '' : 's'}`;
}

export function ParticipantsSummary({
  participants,
  onExpand,
}: {
  participants: MeetingParticipant[];
  onExpand: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onExpand}
      aria-expanded={false}
      className="flex w-full items-center gap-2.5 rounded-[3px] px-1 py-0.5 text-left transition-colors hover:bg-accent focus:outline-none focus-visible:ring-2 focus-visible:ring-ring"
    >
      <AvatarStack attendees={asAttendees(participants)} size="sm" max={4} />
      <span className="min-w-0 flex-1 truncate text-xs text-muted-foreground">
        {participantSummaryText(participants)}
      </span>
      <span className="flex-shrink-0 text-xs font-medium text-muted-foreground">Show all</span>
    </button>
  );
}
