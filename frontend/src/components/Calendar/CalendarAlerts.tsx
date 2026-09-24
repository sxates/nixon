'use client';

/**
 * "Time to join" calendar alerts (spec 0008, P2 frontend — the key piece).
 *
 * App-wide background component (mounted in layout.tsx alongside MeetingAutoDetect)
 * that fires ONE native macOS notification a short lead time before each
 * upcoming calendar meeting starts, so a user in back-to-back calls gets a
 * heads-up even when Nixon is backgrounded.
 *
 * Behaviour:
 *   - Polls `api_get_upcoming_meetings` every 60s, but ONLY when a calendar (EventKit
 *     or Google) is connected (cheap no-op otherwise — we re-check status each tick so a
 *     just-granted permission starts working without a reload).
 *   - Two alerts per meeting (specs/0068 W3): one at T-5, and one as it starts. The
 *     second is skipped while Nixon is already recording.
 *   - The start banner is SCHEDULED with macOS when the T-5 alert fires (specs/0075 W2),
 *     so it arrives on time even when App Nap throttles this webview's timers (ticks were
 *     observed 61–181s apart, which used to skip the start alert's 60s window entirely).
 *     See "The start banner" below.
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
  isAnyCalendarConnected,
  tryGetUpcomingMeetings,
  joinAndRecord,
  formatClockTime,
} from '@/lib/calendar';
import {
  notify,
  cancelPending,
  focusMainWindow,
  CATEGORY_MEETING,
  CATEGORY_PREP,
  CATEGORY_RECORD,
} from '@/lib/osNotification';
import { prepRouteForEvent } from '@/lib/prep';
import { useRecordingState } from '@/contexts/RecordingStateContext';
import { useSidebar } from '@/components/Sidebar/SidebarProvider';
import { GOOGLE_CALENDAR_SYNCED_EVENT } from '@/lib/googleCalendar';
import { safeListen } from '@/lib/safe-listen';

/** How long before start to alert. Default: 2 minutes. */
// Fire at T-5 to match the scheduled meeting's record window (specs/0038 feedback —
// the "-5 minute mark"), up from the old T-2.
const LEAD_MS = 5 * 60 * 1000;
/**
 * The second alert, at the top of the hour (specs/0068 W3, the ask 0067 carried).
 *
 * Five minutes of warning is the wrong amount for the common case: you see it, you finish
 * the thing you were doing, and by the time the meeting starts the banner is gone. This one
 * arrives as the meeting does, and it is skipped entirely while Nixon is already recording.
 *
 * ## The start banner (specs/0075 W2)
 *
 * Normally it is not fired by a tick at all. When the T-5 alert fires, the start banner is
 * handed to macOS with `deliverAtMs = startsAt` under its own id, `meeting-start-<id>` (the
 * T-5 banner's `meeting-<id>` delivery must not replace it). macOS delivers it on time
 * whatever this webview's timers are doing. After that, each tick keeps it honest: a moved
 * meeting is rescheduled (same id replaces), a meeting that leaves the list before it starts
 * (hidden, deleted, moved out of range) has it cancelled, and starting a recording cancels
 * every pending one.
 *
 * The tick's own start alert (inside `START_WINDOW_MS`) is the backstop for a meeting with
 * no scheduled banner — first seen inside the last minute, or scheduling failed. It uses the
 * SAME `meeting-start-<id>` id, so if a stale request is still pending (a reload, a meeting
 * moved into the last minute) the in-app one replaces it rather than doubling it.
 */
const START_WINDOW_MS = 60 * 1000;
/**
 * Don't alert meetings whose start already passed by more than this. The backend returns
 * meetings that started this recently only because we ask it to (`includeStartedWithinMs`);
 * before specs/0075 it returned none, so this grace was dead code.
 */
const PAST_GRACE_MS = 120 * 1000;
/** Poll cadence. */
const POLL_INTERVAL_MS = 60 * 1000;
/** sessionStorage key holding the ids we've already alerted this session. */
const FIRED_KEY = 'nixon-calendar-alerts-fired';
/** sessionStorage key holding the start banners handed to macOS: meeting id → startsAt. */
const SCHEDULED_KEY = 'nixon-calendar-alerts-scheduled';

/** The start banner's notification id — distinct from the T-5 banner's `meeting-<id>`. */
export function startNotificationId(meetingId: string): string {
  return `meeting-start-${meetingId}`;
}

/**
 * A start banner macOS is holding (or has delivered) for one meeting. `registered` is false
 * for an entry restored after a reload: the request is still pending in macOS, but the
 * press callbacks died with the page, so it is re-scheduled to bring them back.
 */
interface ScheduledStart {
  startsAt: string;
  registered: boolean;
}

function loadScheduled(): Map<string, ScheduledStart> {
  try {
    const raw = sessionStorage.getItem(SCHEDULED_KEY);
    const parsed: unknown = raw ? JSON.parse(raw) : null;
    if (!parsed || typeof parsed !== 'object') return new Map();
    return new Map(
      Object.entries(parsed as Record<string, unknown>)
        .filter((e): e is [string, string] => typeof e[1] === 'string')
        .map(([id, startsAt]) => [id, { startsAt, registered: false }]),
    );
  } catch {
    return new Map();
  }
}

function persistScheduled(scheduled: Map<string, ScheduledStart>): void {
  try {
    const plain = Object.fromEntries([...scheduled].map(([id, e]) => [id, e.startsAt]));
    sessionStorage.setItem(SCHEDULED_KEY, JSON.stringify(plain));
  } catch {
    /* sessionStorage unavailable — the in-memory map still holds for the session */
  }
}

/** Withdraw every start banner still pending (a recording started: nothing to offer). */
function cancelAllScheduledStarts(scheduled: Map<string, ScheduledStart>): void {
  if (scheduled.size === 0) return;
  for (const id of scheduled.keys()) void cancelPending(startNotificationId(id));
  scheduled.clear();
  persistScheduled(scheduled);
}

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
 *
 * `deliverAtMs` schedules the (start) banner with macOS instead of showing it now. Resolves
 * to whether macOS took it.
 */
async function alertMeeting(
  meeting: UpcomingMeeting,
  start: Date,
  joinAndRecord: () => void,
  openPrep: () => void,
  starting: boolean,
  deliverAtMs?: number,
): Promise<boolean> {
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

  return notify({
    title,
    body,
    category,
    // One id per meeting and alert: the in-app start alert and the scheduled one share
    // `meeting-start-<id>`, so one replaces the other rather than stacking a second banner.
    id: starting ? startNotificationId(meeting.id) : `meeting-${meeting.id}`,
    deliverAtMs,
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
  // Start banners handed to macOS, keyed by meeting id (see "The start banner").
  const scheduledRef = useRef<Map<string, ScheduledStart>>(new Map());
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
    // A recording started, by any path: "starting now" has nothing to offer any more, so
    // withdraw every start banner still pending. (The start key is left unfired: if the
    // recording stops before the meeting starts, the next tick schedules it again.)
    if (isRecording && !isRecordingRef.current) cancelAllScheduledStarts(scheduledRef.current);
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
    scheduledRef.current = loadScheduled();
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

    const fired = firedRef.current;
    const scheduled = scheduledRef.current;

    /** Fire whichever of the two alerts is due for this meeting. True if `fired` changed. */
    const fireDueAlert = (meeting: UpcomingMeeting, start: Date, msUntil: number): boolean => {
      if (msUntil > LEAD_MS || msUntil < -PAST_GRACE_MS) return false;
      // Which of the two alerts is due. Inside the last minute it is the start alert,
      // whether or not the lead one ever fired — a meeting that first appeared at T-30s
      // should get "starting now", not "in 0 min".
      const starting = msUntil <= START_WINDOW_MS;
      const key = `${meeting.id}@${starting ? 'start' : 'lead'}`;
      if (fired.has(key)) return false;
      // Mark fired BEFORE awaiting the notification so a slow notify() can't let the next
      // tick double-fire the same meeting.
      fired.add(key);
      if (starting) {
        // Already recording: the start alert has nothing to offer. (The lead one still
        // fires — it is about the NEXT meeting.)
        if (isRecordingRef.current) return true;
        // macOS is delivering (or has delivered) this very banner at the start time. A
        // restored-after-reload entry still pending falls through: the in-app alert
        // replaces it under the same id, which is what brings its callbacks back.
        const entry = scheduled.get(meeting.id);
        if (entry && entry.startsAt === meeting.startsAt && (entry.registered || msUntil <= 0)) {
          return true;
        }
        if (entry) {
          scheduled.delete(meeting.id);
          persistScheduled(scheduled);
        }
      }
      const m = meeting;
      void alertMeeting(m, start, () => triggerJoinAndRecord(m), () => openPrep(m), starting);
      return true;
    };

    /**
     * Keep the scheduled start banner in step with the meeting: schedule it once the lead
     * alert has fired, re-schedule it when the meeting moved (same id replaces the pending
     * request) or when a reload lost its callbacks.
     */
    const syncStartBanner = (meeting: UpcomingMeeting, start: Date, msUntil: number) => {
      const entry = scheduled.get(meeting.id);
      const wanted =
        fired.has(`${meeting.id}@lead`) &&
        !fired.has(`${meeting.id}@start`) &&
        !isRecordingRef.current &&
        msUntil > START_WINDOW_MS;
      if (!wanted) return;
      if (entry && entry.registered && entry.startsAt === meeting.startsAt) return;

      scheduled.set(meeting.id, { startsAt: meeting.startsAt, registered: true });
      persistScheduled(scheduled);
      const m = meeting;
      void alertMeeting(
        m,
        start,
        () => triggerJoinAndRecord(m),
        () => openPrep(m),
        true,
        start.getTime(),
      ).then((ok) => {
        if (ok && !scheduled.has(m.id)) {
          // Withdrawn (a recording started, the meeting vanished) while the request was
          // still on its way to macOS: the cancel may have landed first, so cancel again.
          void cancelPending(startNotificationId(m.id));
        } else if (!ok && scheduled.get(m.id)?.startsAt === m.startsAt) {
          // macOS did not take it: forget it, so the in-app start alert is not suppressed.
          scheduled.delete(m.id);
          persistScheduled(scheduled);
        }
      });
    };

    const tick = async () => {
      if (cancelled) return;
      // Cheap gate: do nothing without a calendar. EITHER source counts (specs/0074 W5) —
      // gating on EventKit alone left Google-only users with no alerts at all.
      const connected = await isAnyCalendarConnected();
      if (cancelled || !connected) return;

      // `null` is a failed read, not an empty calendar — it must not cancel anything.
      const meetings = await tryGetUpcomingMeetings(12, { includeStartedWithinMs: PAST_GRACE_MS });
      if (cancelled || meetings === null) return;

      const now = Date.now();
      let changed = false;
      const listed = new Set<string>();
      for (const meeting of meetings) {
        const start = new Date(meeting.startsAt);
        if (Number.isNaN(start.getTime())) continue;
        listed.add(meeting.id);
        const msUntil = start.getTime() - now;
        if (fireDueAlert(meeting, start, msUntil)) changed = true;
        syncStartBanner(meeting, start, msUntil);
      }
      if (changed) persistFired(fired);

      // A meeting that left the list before it started was hidden, deleted or moved out
      // of range: withdraw its start banner. One that already started just drops out once
      // it is past the grace — macOS delivered its banner.
      let scheduledChanged = false;
      for (const [id, entry] of scheduled) {
        if (listed.has(id)) continue;
        if (Date.parse(entry.startsAt) > now) void cancelPending(startNotificationId(id));
        scheduled.delete(id);
        scheduledChanged = true;
      }
      if (scheduledChanged) persistScheduled(scheduled);
    };

    void tick(); // fire immediately so meetings already within the window alert on load
    const interval = window.setInterval(() => void tick(), POLL_INTERVAL_MS);
    // A finished Google sync may have moved, added or removed a meeting: look now rather
    // than up to a minute later.
    const disposeSynced = safeListen(GOOGLE_CALENDAR_SYNCED_EVENT, () => void tick());
    return () => {
      cancelled = true;
      window.clearInterval(interval);
      disposeSynced();
    };
  }, []);

  return null;
}
