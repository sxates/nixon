/**
 * specs/0087 — parse a `nixon://meeting/<id>?t=<seconds>` link (or the dev build's
 * `nixon-dev://`). The OS only hands this app its own scheme, but anything that is not a
 * meeting link is still refused: links only ever navigate to a meeting.
 */
export interface MeetingLink {
  meetingId: string;
  /** Seconds into the recording, or null when absent or not a finite number >= 0. */
  t: number | null;
}

const SCHEMES = new Set(['nixon:', 'nixon-dev:']);

export function parseMeetingLink(url: string): MeetingLink | null {
  let parsed: URL;
  try {
    parsed = new URL(url);
  } catch {
    return null;
  }
  if (!SCHEMES.has(parsed.protocol) || parsed.hostname !== 'meeting') return null;

  const segments = parsed.pathname.split('/').slice(1);
  if (segments.length !== 1 || segments[0] === '') return null;
  let meetingId: string;
  try {
    meetingId = decodeURIComponent(segments[0]);
  } catch {
    return null;
  }
  if (!meetingId) return null;

  const raw = parsed.searchParams.get('t');
  const n = raw === null || raw.trim() === '' ? NaN : Number(raw);
  return { meetingId, t: Number.isFinite(n) && n >= 0 ? n : null };
}
