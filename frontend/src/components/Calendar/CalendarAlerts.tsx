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
 *   - Two alerts per meeting (specs/0068 W3): one at T-5, and one as it starts. The
 *     second reuses the first's notification id, so macOS replaces the banner instead of
 *     stacking a second one, and it is skipped while Nixon is already recording.
 *   - De-dupe by `${meetingId}@lead` / `${meetingId}@start`: each is fired at most once
 *     per session. Fired keys are kept in-memory AND mirrored to sessionStorage so a
 *     reload (Next fast-refresh in dev, or a real reload) doesn't re-fire alerts for
 *     meetings that already passed.
 *   - The banner carries one button, **Join & Record**: it opens the Zoom client (deep
 *     link, https fallback) AND starts a recording bound to the event — unless Nixon is
 *     already recording, in which case the record half is skipped
 *     (`handleRecordingToggle` also no-ops while recording, so we never double-start).
 *     One button rather than two because macOS 26 hides a second one behind an "Options"
 *     menu; tapping the banner body opens Nixon, where joining without recording is one
 *     click away. The button only began working in specs/0068 — before that the plugin it
 *     went through had no action support on macOS at all.
 *
 * Degrades gracefully: on a bare dev binary calendar access is never authorized and there
 * is no notification centre to deliver to (specs/0068), so this is a silent no-op.
 */

import { useEffect, useRef } from 'react';
import { useRouter } from 'next/navigation';
import { toast } from 'sonner';
import {
  type UpcomingMeeting,
  getCalendarAccessStatus,
  getUpcomingMeetings,
  joinAndRecord,
  formatClockTime,
} from '@/lib/calendar';
import {
  notify,
  focusMainWindow,
  CATEGORY_MEETING,
  CATEGORY_PREP,
  CATEGORY_RECORD,
} from '@/lib/osNotification';
import { prepRouteForEvent } from '@/lib/prep';
import { useRecordingState } from '@/contexts/RecordingStateContext';
import { useSidebar } from '@/components/Sidebar/SidebarProvider';

/** How long before start to alert. Default: 2 minutes. */
// Fire at T-5 to match the scheduled meeting's record window (specs/0038 feedback —
// the "-5 minute mark"), up from the old T-2.
const LEAD_MS = 5 * 60 * 1000;
/**
 * The second alert, at the top of the hour (specs/0068 W3, the ask 0067 carried).
 *
 * Five minutes of warning is the wrong amount for the common case: you see it, you finish
 * the thing you were doing, and by the time the meeting starts the banner is gone. This one
 * arrives as the meeting does. It is a *replacement*, not an addition — it reuses the same
 * notification id, so macOS swaps the "in 5 min" banner for it rather than stacking two —
 * and it is skipped entirely while Nixon is already recording.
 */
const START_WINDOW_MS = 60 * 1000;
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
  openPrep: () => void,
  starting: boolean,
): Promise<void> {
  // The caller has already decided which of the two alerts this is; re-deriving it from the
  // clock here would let the wording disagree with that decision at the boundary (50s out
  // rounds to "in 1 min" while the caller is firing the start alert).
  const minutesUntil = Math.max(1, Math.round((start.getTime() - Date.now()) / 60000));
  const title = starting
    ? `${meeting.title} starting now`
    : `${meeting.title} in ${minutesUntil} min`;
  const bodyParts = [formatClockTime(start)];
  if (meeting.calendarName) bodyParts.push(meeting.calendarName);
  const body = bodyParts.join(' · ');

  // Each alert carries the one thing worth doing at that moment — a banner can only show
  // a single button (see `CATEGORY_MEETING` on the Rust side). Five minutes out that is
  // **Prep**: you are not joining yet, you are working out what the meeting is for. At the
  // top of the hour it is **Join & Record**, or plain **Record** when there is no link to
  // open — Nixon records either way, since system audio and mic do not care what the call
  // is on (the specs/0038 point about link-less Teams/Meet meetings).
  const category = !starting
    ? CATEGORY_PREP
    : meeting.zoomUrl
      ? CATEGORY_MEETING
      : CATEGORY_RECORD;

  await notify({
    title,
    body,
    category,
    // One id per meeting: a later alert for the same meeting replaces its banner rather
    // than stacking a second one.
    id: `meeting-${meeting.id}`,
    userInfo: { meetingId: meeting.id },
    onJoinAndRecord: joinAndRecord,
    // The link-less category's only button; same intent.
    onRecord: joinAndRecord,
    onPrep: openPrep,
    // Tapping the body opens Nixon at whatever the button would have done something
    // about: the Prep tab for the warning, and the app itself as the meeting starts —
    // the route to joining WITHOUT recording, which the single button cannot offer.
    onOpen: starting ? () => void focusMainWindow() : openPrep,
  });
}

export default function CalendarAlerts() {
  // In-memory fired set, seeded from sessionStorage so a reload doesn't re-fire.
  const firedRef = useRef<Set<string>>(new Set());
  const router = useRouter();
  const routerRef = useRef(router);

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
    routerRef.current = router;
  }, [router]);
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

    // "Prep" on the five-minute warning: mint (or reuse) the scheduled meeting row for
    // this occurrence and open its Prep tab. Nixon is behind something when the banner is
    // pressed, so bring it forward first.
    const openPrep = async (meeting: UpcomingMeeting) => {
      void focusMainWindow();
      try {
        routerRef.current.push(
          await prepRouteForEvent({
            id: meeting.id,
            title: meeting.title,
            startsAt: meeting.startsAt,
            externalId: meeting.externalId,
          }),
        );
      } catch (error) {
        console.error('[CalendarAlerts] could not open prep:', error);
        toast.error('Could not open prep for this meeting');
      }
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
        const start = new Date(meeting.startsAt);
        if (Number.isNaN(start.getTime())) continue;
        const msUntil = start.getTime() - now;
        if (msUntil > LEAD_MS) continue;
        if (msUntil < -PAST_GRACE_MS) continue;

        // Which of the two alerts is due. Inside the last minute it is the start alert,
        // whether or not the lead one ever fired — a meeting that first appeared at T-30s
        // should get "starting now", not "in 0 min".
        const starting = msUntil <= START_WINDOW_MS;
        const key = `${meeting.id}@${starting ? 'start' : 'lead'}`;
        if (firedRef.current.has(key)) continue;
        // Already recording: the start alert has nothing to offer, and the lead one would
        // be interrupting a meeting to announce it.
        if (starting && isRecordingRef.current) {
          firedRef.current.add(key);
          changed = true;
          continue;
        }

        // Mark fired BEFORE awaiting the notification so a slow notify() can't
        // let the next 60s tick double-fire the same meeting.
        firedRef.current.add(key);
        changed = true;
        const m = meeting;
        void alertMeeting(m, start, () => triggerJoinAndRecord(m), () => openPrep(m), starting);
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
