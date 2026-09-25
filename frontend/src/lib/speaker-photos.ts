import type { MeetingAttendee, MeetingSpeaker, Person } from '@/types';

/** speakerKey → cached directory photo (`data:` URI). Only speakers that HAVE one appear. */
export type SpeakerPhotoMap = ReadonlyMap<string, string>;

const norm = (email: string | null | undefined) => email?.trim().toLowerCase() || null;

/**
 * Resolve each meeting speaker's Google directory photo from data the meeting view already
 * holds (`useSpeakers`: the speakers, the calendar attendees and the ranked people list — all
 * three carry `photoDataUri` joined from `attendee_photos` by the backend). No IPC: the
 * transcript can have 1,000+ rows, so the map is built once per view and every row does one
 * `Map.get`.
 *
 * Precedence per speaker: the linked Person (`personId`) → a Person or attendee with the
 * speaker's email → for the local "You" speaker with no email, the attendee marked
 * `isCurrentUser`. A speaker with none of those simply has no entry (initials fallback).
 */
export function buildSpeakerPhotoMap(
  speakers: readonly MeetingSpeaker[],
  attendees: readonly MeetingAttendee[],
  people: readonly Person[],
): SpeakerPhotoMap {
  const map = new Map<string, string>();
  if (speakers.length === 0) return map;

  const byPersonId = new Map<string, string>();
  const byEmail = new Map<string, string>();
  for (const p of people) {
    if (!p.photoDataUri) continue;
    byPersonId.set(p.id, p.photoDataUri);
    const e = norm(p.email);
    if (e) byEmail.set(e, p.photoDataUri);
  }
  let selfPhoto: string | null = null;
  for (const a of attendees) {
    if (!a.photoDataUri) continue;
    const e = norm(a.email);
    if (e && !byEmail.has(e)) byEmail.set(e, a.photoDataUri);
    if (a.isCurrentUser && !selfPhoto) selfPhoto = a.photoDataUri;
  }

  for (const s of speakers) {
    const email = norm(s.email);
    const photo =
      (s.personId ? byPersonId.get(s.personId) : undefined) ??
      (email ? byEmail.get(email) : undefined) ??
      (s.isLocal && !email ? selfPhoto : null);
    if (photo) map.set(s.speakerKey, photo);
  }
  return map;
}
