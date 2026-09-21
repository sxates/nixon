'use client';

/**
 * ScheduledRecordControl (specs/0038 dogfood feedback; reworked in specs/0041 WS3) — the
 * compact record control in the header of a CALENDAR-LINKED meeting's detail page,
 * sitting next to the "…" overflow menu. Two modes, keyed off the row's state:
 *
 *  - **Start** (a `scheduled` occurrence, or a calendar row that never actually
 *    recorded): starts a recording bound to THIS event via `joinAndRecord`, so the
 *    recording adopts the event's identity + roster (the scheduled row is promoted to
 *    'recorded'). The event's join link is resolved from the day agenda (the same
 *    source the Home agenda uses) and passed through, so clicking also OPENS the call
 *    (Zoom/Meet/Teams); with no link it records only.
 *  - **Continue** (the occurrence already recorded — origin='recorded' with a recording
 *    folder or transcripts, e.g. a false start you stopped): routes through the
 *    specs/0037 resume path (same `meeting_id`, new segment) so retrying NEVER mints a
 *    duplicate meeting row. If the event has a join link it re-opens the call first.
 *
 * Layout is deliberately lightweight (a header chip + button, not a full-width banner):
 *  - The record button shows from T-5 onward (and stays for a call that starts late).
 *    **Manual entries** (`isManualEntry`) skip this window entirely — the button is
 *    always there, however far out the entry's date/time is (specs/0069b followup):
 *    unlike a calendar event, the time is the user's own choice, not an invite's.
 *  - The countdown chip is shown while the meeting is upcoming or recently started, and
 *    HIDDEN once it's well in the past (> 1h ago) — a stale scheduled row shouldn't shout
 *    "Started 579m ago"; it just keeps the button. Unaffected by `isManualEntry`.
 *  - Continue mode hides entirely once > 1h past start — the "…" menu's "Continue
 *    recording" item covers late appends; the header button is the false-start retry.
 */

import { useEffect, useState } from 'react';
import { Mic, Radio } from 'lucide-react';
import { useRouter } from 'next/navigation';
import { useSidebar } from '@/components/Sidebar/SidebarProvider';
import { useRecordingState } from '@/contexts/RecordingStateContext';
import {
  joinAndRecord,
  openZoomMeeting,
  resolveOccurrenceJoinUrl,
  resolveRecordingStartsAt,
  formatRelativeStart,
  JOIN_AND_RECORD_DELAY_MS,
} from '@/lib/calendar';
import { armResumeRecording } from '@/lib/resume-recording';

/** The record button opens this long before start (matches the Today view's T-5 "now" phase). */
const JOIN_WINDOW_MS = 5 * 60 * 1000;
/** Past this far after start, drop the countdown text (keep only the start-mode button). */
const STALE_AFTER_MS = 60 * 60 * 1000;

export function ScheduledRecordControl({
  meetingId,
  startsAt,
  title,
  calendarEventId,
  seriesKey,
  origin,
  hasTranscripts,
  folderPath,
  isManualEntry = false,
}: {
  /** The meeting row id — continue mode resumes INTO this row (specs/0037). */
  meetingId: string;
  /** The scheduled occurrence start (ISO-8601) — the row's `created_at`. */
  startsAt: string;
  title: string;
  /** The event's `calendar_event_id` — binds the recording to this event's roster. */
  calendarEventId: string;
  /** The recurring-series key, if any (groups the recording into its series). */
  seriesKey: string | null;
  /** Row origin: 'scheduled' (prep placeholder) or 'recorded' (already recorded once). */
  origin: 'scheduled' | 'recorded';
  /** Whether the row already has transcript segments (drives continue-vs-start). */
  hasTranscripts: boolean;
  /** The row's recording folder, if any — required by the specs/0037 resume path. */
  folderPath: string | null;
  /**
   * True for a `nixon-manual:` entry (backend-computed, specs/0069b). Manual entries
   * bypass the T-5 `canRecord` window — always own the button — and, when recorded
   * before their scheduled time, date the row to `now` instead (see
   * {@link resolveRecordingStartsAt}). Calendar-linked meetings are unaffected.
   */
  isManualEntry?: boolean;
}) {
  const router = useRouter();
  const { handleRecordingToggle } = useSidebar();
  const { isRecording } = useRecordingState();

  // Minute-granular tick (no sub-minute precision needed), mirroring the Today view clock.
  const [now, setNow] = useState(() => new Date());
  useEffect(() => {
    const id = window.setInterval(() => setNow(new Date()), 60_000);
    return () => window.clearInterval(id);
  }, []);

  // The event's join link, resolved from the day agenda for the occurrence's day —
  // the same `api_get_day_agenda` source that gives the Home agenda its `zoomUrl`
  // (specs/0041 WS3). Null while resolving / when the event has no link, in which
  // case the control degrades to record-only (today's behavior).
  const [joinUrl, setJoinUrl] = useState<string | null>(null);
  useEffect(() => {
    // Stale gate (same math as showCountdown below, from a fresh clock): resolving the
    // join URL walks the whole day agenda (`api_get_day_agenda` can trigger a Google
    // network sync), which is pure waste for an occurrence > 1h past start — there's no
    // call left to join, and continue mode doesn't even render then. The click handler
    // reads `joinUrl` at click time, so a skipped fetch just degrades start mode to
    // record-only ("Start & record"). NOTE: don't gate on `canRecord` — it's true for
    // ALL past meetings, however old.
    const startMs = new Date(startsAt).getTime();
    if (Number.isNaN(startMs) || startMs - Date.now() <= -STALE_AFTER_MS) return;
    let cancelled = false;
    void resolveOccurrenceJoinUrl({
      calendarEventId,
      occurrenceStart: startsAt,
      meetingId,
      seriesKey,
    }).then((url) => {
      if (!cancelled) setJoinUrl(url);
    });
    return () => {
      cancelled = true;
    };
  }, [calendarEventId, startsAt, meetingId, seriesKey]);

  // Continue mode is arming when the call was just (re)opened and the resume start is
  // scheduled — disables the button so a double-click can't arm two resumes.
  const [isArming, setIsArming] = useState(false);

  const start = new Date(startsAt);
  if (Number.isNaN(start.getTime())) return null;

  // The occurrence already recorded once (a false start, or an append): retrying must
  // CONTINUE into the same meeting row, never create a second one (specs/0041 WS3).
  // A recorded row with neither a folder nor transcripts never actually captured
  // anything — for that one, start mode is safe: `api_create_meeting`'s adoption
  // dedupe (find_adoptable_calendar_meeting) reuses the empty row.
  const mode: 'start' | 'continue' =
    origin === 'recorded' && (hasTranscripts || !!folderPath) ? 'continue' : 'start';

  const msUntil = start.getTime() - now.getTime();
  // Manual entries always own the button (specs/0069b followup) — the T-5 window only
  // applies to calendar-linked meetings, whose time isn't the user's to choose.
  const canRecord = isManualEntry || msUntil <= JOIN_WINDOW_MS; // T-5 onward, incl. after start
  const showCountdown = msUntil > -STALE_AFTER_MS; // hidden once > 1h past

  // Continue mode is a header convenience for the around-the-meeting retry; once the
  // occurrence is stale (> 1h past start) the "…" menu's Continue recording owns it.
  if (mode === 'continue' && !showCountdown) return null;

  let countdownLabel = '';
  if (showCountdown) {
    if (msUntil > 0) {
      countdownLabel = `Starts ${formatRelativeStart(start, now)}`; // "Starts in 25m"
    } else if (msUntil > -60_000) {
      countdownLabel = 'Starting now';
    } else {
      countdownLabel = `Started ${Math.round(-msUntil / 60_000)}m ago`;
    }
  }

  const onStartRecord = () => {
    // Bind the recording to this calendar event so it keeps the roster, opening the
    // call first when the event has a join link (Zoom/Teams/Meet — we record system
    // audio regardless of platform; no link just records). For a calendar-linked
    // meeting, `startsAt` is re-emitted as strict RFC3339 so `api_create_meeting`
    // dates the row to the occurrence instead of silently falling back to now
    // (specs/0041 WS3 timestamp hygiene). For a manual entry recorded ahead of its
    // own scheduled time, `resolveRecordingStartsAt` sends `now` instead — otherwise
    // `redate_scheduled_meeting` would file the recording under a day that hasn't
    // happened yet (specs/0069b followup).
    void joinAndRecord(
      {
        id: calendarEventId,
        title,
        zoomUrl: joinUrl,
        startsAt: isManualEntry ? resolveRecordingStartsAt(startsAt, now) : start.toISOString(),
        seriesKey,
      },
      isRecording,
      handleRecordingToggle,
    );
  };

  const onContinueRecording = () => {
    if (isRecording || isArming) return;
    // Same mechanism as the "…" menu's Continue recording (specs/0037): arm the
    // resume stash and let `/record`'s `useRecordingStart` consume it — it appends
    // into THIS meeting id + folder (resolving the folder itself when null, and
    // aborting loudly if the row has none). Never a new meeting row.
    const proceed = () => {
      armResumeRecording({ meetingId, folderPath, meetingName: title });
      router.push('/record');
    };
    if (joinUrl) {
      // Re-open the call first, then start the tap after the same settle delay
      // Join & Record uses (creating the Core Audio tap while the client's audio
      // engine initializes can hang it — see JOIN_AND_RECORD_DELAY_MS).
      setIsArming(true);
      void openZoomMeeting(joinUrl);
      window.setTimeout(proceed, JOIN_AND_RECORD_DELAY_MS);
    } else {
      proceed();
    }
  };

  const isContinue = mode === 'continue';
  const buttonLabel = isRecording
    ? 'Recording…'
    : isContinue
      ? isArming
        ? 'Opening call…'
        : 'Continue recording'
      : joinUrl
        ? 'Join & record'
        : 'Start & record';
  const buttonTitle = isRecording
    ? 'A recording is already in progress'
    : isContinue
      ? 'Resume recording into this meeting — new audio adds to the existing transcript, no duplicate meeting'
      : joinUrl
        ? 'Open the call and start recording — this meeting keeps its calendar attendees'
        : 'Start recording — this meeting keeps its calendar attendees';

  return (
    <div className="mt-1 flex flex-shrink-0 items-center gap-2">
      {showCountdown && countdownLabel && (
        <span className="whitespace-nowrap text-xs text-muted-foreground">{countdownLabel}</span>
      )}
      {canRecord && (
        <button
          type="button"
          onClick={isContinue ? onContinueRecording : onStartRecord}
          disabled={isRecording || (isContinue && isArming)}
          title={buttonTitle}
          className="inline-flex items-center gap-1.5 rounded-lg bg-brand px-3 py-1.5 text-sm font-semibold text-brand-foreground transition-colors hover:bg-brand/90 focus:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-60"
        >
          {isContinue ? <Radio size={14} aria-hidden="true" /> : <Mic size={14} aria-hidden="true" />}
          {buttonLabel}
        </button>
      )}
    </div>
  );
}
