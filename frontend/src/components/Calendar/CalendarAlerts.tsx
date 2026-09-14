'use client';

/**
 * "Time to join" calendar alerts (spec 0008, P2 frontend — the key piece).
 *
 * App-wide background component (mounted in layout.tsx alongside ZoomAutoDetect)
 * that fires ONE native macOS notification a short lead time before each
 * upcoming calendar meeting starts, so a user in back-to-back calls gets a
 * heads-up even when Nixon is backgrounded.
 *
 * Behaviour:
 *   - Polls `api_get_upcoming_meetings` every 60s, but ONLY when calendar access
 *     is authorized (cheap no-op otherwise — we re-check status each tick so a
 *     just-granted permission starts working without a reload).
 *   - For each meeting whose start is within LEAD_MS ahead (and not already more
 *     than PAST_GRACE_MS past), fire a notification once. The window is
 *     "approaching OR already inside the lead window" so a meeting that's already
 *     <2 min out on first load still alerts.
 *   - De-dupe by meeting id: each id is alerted at most once per session. Fired
 *     ids are kept in-memory AND mirrored to sessionStorage so a reload (Next
 *     fast-refresh in dev, or a real reload) doesn't re-fire alerts for meetings
 *     that already passed.
 *   - The notification's PRIMARY action is "Join & Record" (spec 0008, P3): via
 *     osNotification's generic Record/Ignore buttons reused as Join & Record /
 *     dismiss, plus a body-tap that does the same. Pressing it opens the Zoom
 *     client (deep link, https fallback), focuses Nixon, AND starts recording via
 *     the existing auto-start path — unless Nixon is already recording, in which
 *     case the record half is skipped (P1's auto-detect / a manual start may have
 *     already begun a recording; handleRecordingToggle also no-ops while
 *     recording, so we never double-start).
 *
 * Degrades gracefully: on a bare dev binary calendar access is never authorized
 * and notifications can't be delivered, so this is a silent no-op (no crash).
 */

import { useEffect, useRef } from 'react';
import {
  type UpcomingMeeting,
  getCalendarAccessStatus,
  getUpcomingMeetings,
  joinAndRecord,
  formatClockTime,
} from '@/lib/calendar';
import { notify, focusMainWindow } from '@/lib/osNotification';
import { useRecordingState } from '@/contexts/RecordingStateContext';
import { useSidebar } from '@/components/Sidebar/SidebarProvider';

/** How long before start to alert. Default: 2 minutes. */
// Fire at T-5 to match the scheduled meeting's record window (specs/0038 feedback —
// the "-5 minute mark"), up from the old T-2.
const LEAD_MS = 5 * 60 * 1000;
/** Don't alert meetings whose start already passed by more than this. */
const PAST_GRACE_MS = 60 * 1000;
/** Poll cadence. */
const POLL_INTERVAL_MS = 60 * 1000;
/** sessionStorage key holding the ids we've already alerted this session. */
const FIRED_KEY = 'nixon-calendar-alerts-fired';

function loadFired(): Set<string> {
  try {
    const raw = sessionStorage.getItem(FIRED_KEY);
    if (!raw) return new Set();
    const parsed = JSON.parse(raw);
    return Array.isArray(parsed) ? new Set(parsed as string[]) : new Set();
  } catch {
    return new Set();
  }
}

function persistFired(fired: Set<string>): void {
  try {
    sessionStorage.setItem(FIRED_KEY, JSON.stringify([...fired]));
  } catch {
    /* sessionStorage unavailable — in-memory de-dupe still holds for the session */
  }
}

/**
 * Fire the "time to join" notification for one meeting.
 *
 * `joinAndRecord` opens the Zoom client and (unless already recording) starts a
 * Nixon recording — it's the notification's primary action and body-tap.
 */
async function alertMeeting(
  meeting: UpcomingMeeting,
  start: Date,
  joinAndRecord: () => void,
): Promise<void> {
  const minutesUntil = Math.max(0, Math.round((start.getTime() - Date.now()) / 60000));
  const title =
    minutesUntil <= 0
      ? `${meeting.title} starting now`
      : `${meeting.title} in ${minutesUntil} min`;
  const bodyParts = [formatClockTime(start)];
  if (meeting.calendarName) bodyParts.push(meeting.calendarName);
  const body = bodyParts.join(' · ');

  const primary = () => {
    void focusMainWindow();
    joinAndRecord();
  };
  await notify({
    title,
    body,
    // Always actionable: we record system audio + mic regardless of platform, so the
    // "Record" action is offered even for link-less (Teams/Meet) meetings — which
    // previously got a dead, non-actionable notification (specs/0038 feedback).
    actionable: true,
    // "Record" button + body-tap = start a recording bound to this calendar event.
    onRecord: primary,
    onClick: primary,
  });
}

export default function CalendarAlerts() {
  // In-memory fired set, seeded from sessionStorage so a reload doesn't re-fire.
  const firedRef = useRef<Set<string>>(new Set());

  // Keep the latest recording state + start path in refs so the long-lived
  // notification callbacks (created when an alert fires) always read current
  // values without re-subscribing the poll loop.
  const { isRecording } = useRecordingState();
  const { handleRecordingToggle } = useSidebar();
  const isRecordingRef = useRef(isRecording);
  const handleRecordingToggleRef = useRef(handleRecordingToggle);
  useEffect(() => {
    isRecordingRef.current = isRecording;
  }, [isRecording]);
  useEffect(() => {
    handleRecordingToggleRef.current = handleRecordingToggle;
  }, [handleRecordingToggle]);

  useEffect(() => {
    firedRef.current = loadFired();
    let cancelled = false;

    // Start a Nixon recording then open Zoom (record-first ordering avoids a
    // Core Audio tap vs. Zoom audio-init collision that hangs the client),
    // re-checking isRecording at click time to avoid a double-start.
    const triggerJoinAndRecord = (meeting: UpcomingMeeting) => {
      // Spec 0015: the recording adopts this calendar event's identity (title + id).
      void joinAndRecord(
        // Pass startsAt so the recording dates to (and adopts the scheduled row for)
        // this occurrence, keeping its attendees — not just an ad-hoc row.
        { id: meeting.id, title: meeting.title, zoomUrl: meeting.zoomUrl, startsAt: meeting.startsAt },
        isRecordingRef.current,
        handleRecordingToggleRef.current,
      );
    };

    const tick = async () => {
      if (cancelled) return;
      // Cheap gate: do nothing unless calendar is authorized.
      const status = await getCalendarAccessStatus();
      if (cancelled || status !== 'authorized') return;

      const meetings = await getUpcomingMeetings();
      if (cancelled) return;

      const now = Date.now();
      let changed = false;
      for (const meeting of meetings) {
        if (firedRef.current.has(meeting.id)) continue;
        const start = new Date(meeting.startsAt);
        if (Number.isNaN(start.getTime())) continue;
        const msUntil = start.getTime() - now;
        // Within (or already inside) the lead window, and not stale.
        if (msUntil > LEAD_MS) continue;
        if (msUntil < -PAST_GRACE_MS) continue;

        // Mark fired BEFORE awaiting the notification so a slow notify() can't
        // let the next 60s tick double-fire the same meeting.
        firedRef.current.add(meeting.id);
        changed = true;
        const m = meeting;
        void alertMeeting(m, start, () => triggerJoinAndRecord(m));
      }
      if (changed) persistFired(firedRef.current);
    };

    void tick(); // fire immediately so meetings already within the window alert on load
    const interval = window.setInterval(() => void tick(), POLL_INTERVAL_MS);
    return () => {
      cancelled = true;
      window.clearInterval(interval);
    };
  }, []);

  return null;
}
