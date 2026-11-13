/**
 * specs/0087 — parse a `nixon://meeting/<id>?t=<seconds>` link (or the dev build's
 * `nixon-dev://`). The OS only hands this app its own scheme, but anything that is not a
 * meeting link is still refused: links only ever navigate to a meeting.
 */
import { invoke } from '@tauri-apps/api/core';
import { toast } from 'sonner';

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

interface LinkTarget {
  exists: boolean;
  segmentId: string | null;
}

/**
 * Open a meeting at a moment: resolve the segment through the read-only
 * `api_resolve_meeting_link`, then push the route. Shared by `nixon://` links
 * (DeepLinkListener) and Topics timeline "open at the moment" links. Never throws;
 * failures toast.
 */
export async function openMeetingMoment(
  router: { push: (href: string) => void },
  meetingId: string,
  t: number | null,
): Promise<void> {
  try {
    const target = await invoke<LinkTarget>('api_resolve_meeting_link', { meetingId, t });
    if (!target.exists) {
      toast.error("That meeting isn't in Nixon anymore.");
      return;
    }
    const id = encodeURIComponent(meetingId);
    router.push(
      target.segmentId
        ? `/meeting-details?id=${id}&segment=${encodeURIComponent(target.segmentId)}`
        : `/meeting-details?id=${id}`,
    );
  } catch (e) {
    console.error('[DeepLink] resolve failed:', e);
    toast.error("Nixon couldn't open that link.");
  }
}
