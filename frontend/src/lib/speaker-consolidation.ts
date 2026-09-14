import type { MeetingSpeaker } from '@/types';

/**
 * specs/0019 WS2.4 (note 9) — Auto-consolidate speakers mapped to the same person.
 *
 * Diarization can split one real person across several `speaker_key`s (over-
 * segmentation), and assigning each of them to the same Person leaves the legend
 * showing two "Priya" chips. The durable identity is `people.id`, copied onto each
 * speaker row as `personId` (and `email`) when assigned — so two speaker rows linked
 * to the same person share those values. This collapses such rows into one chip.
 *
 * Grouping anchor, most authoritative first:
 *   1. `personId`  — set the moment a speaker is assigned to a Person; the real key.
 *   2. `email`     — a calendar-attendee assignment sets email but may predate a
 *                    person link; still a reliable identity for the same individual.
 *   3. `speakerKey`— ungrouped fallback (unnamed speakers never consolidate).
 *
 * Order is preserved by first appearance, so the local "You" speaker (listed first by
 * the backend, and normally carrying neither personId nor email) stays put.
 */
export interface ConsolidatedSpeaker {
  /** The representative speaker shown in the legend (first occurrence in the group). */
  primary: MeetingSpeaker;
  /** Every speaker in this identity group, primary first (length >= 1). */
  members: MeetingSpeaker[];
  /** All member `speaker_key`s — corrections fan out across these to stay consolidated. */
  keys: string[];
}

function identityKey(s: MeetingSpeaker): string {
  if (s.personId) return `person:${s.personId}`;
  const email = s.email?.trim().toLowerCase();
  if (email) return `email:${email}`;
  return `key:${s.speakerKey}`;
}

export function consolidateSpeakers(
  speakers: MeetingSpeaker[],
): ConsolidatedSpeaker[] {
  const groups = new Map<string, ConsolidatedSpeaker>();
  const order: string[] = [];

  for (const speaker of speakers) {
    const key = identityKey(speaker);
    const existing = groups.get(key);
    if (existing) {
      existing.members.push(speaker);
      existing.keys.push(speaker.speakerKey);
    } else {
      groups.set(key, {
        primary: speaker,
        members: [speaker],
        keys: [speaker.speakerKey],
      });
      order.push(key);
    }
  }

  return order.map((k) => groups.get(k)!);
}
