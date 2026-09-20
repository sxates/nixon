'use client';

/**
 * "Upcoming" meetings section + "Connect your calendar" prompt for the
 * dashboard (spec 0008, P2 frontend).
 *
 * Self-contained: it reads the calendar access status and, when authorized,
 * fetches upcoming meetings and renders them ABOVE the past-meeting groups.
 * When not authorized it renders an unobtrusive "Connect your calendar" prompt
 * (dismissible, sticky via localStorage). It renders nothing (returns null) when
 * there's nothing to show, so the dashboard layout is unaffected on a dev binary
 * with no calendar access and no upcoming meetings.
 *
 * Refresh: on mount, on window focus, and on a light 5-minute interval. All
 * backend calls degrade gracefully (see lib/calendar.ts) — failures never throw
 * and never block the dashboard.
 */

import { useCallback, useEffect, useState } from 'react';
import { Video, CircleDot } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { ConnectCalendarCard } from '@/components/Calendar/ConnectCalendarCard';
import { useRecordingState } from '@/contexts/RecordingStateContext';
import { useSidebar } from '@/components/Sidebar/SidebarProvider';
import {
  type CalendarAccessStatus,
  type UpcomingMeeting,
  getCalendarAccessStatus,
  getUpcomingMeetings,
  openZoomMeeting,
  joinAndRecord,
  formatClockTime,
  formatRelativeStart,
} from '@/lib/calendar';

const DISMISS_KEY = 'nixon-calendar-connect-dismissed';
const REFRESH_INTERVAL_MS = 5 * 60 * 1000; // 5 minutes

function MeetingRow({ meeting }: { meeting: UpcomingMeeting }) {
  const start = new Date(meeting.startsAt);
  const validStart = !Number.isNaN(start.getTime());
  const title = meeting.title?.trim() || 'Untitled meeting';

  const { isRecording } = useRecordingState();
  const { handleRecordingToggle } = useSidebar();

  // "Join" — open the Zoom client (deep link, https fallback) only.
  const handleJoin = useCallback(() => {
    if (meeting.zoomUrl) void openZoomMeeting(meeting.zoomUrl);
  }, [meeting.zoomUrl]);

  // "Join & Record" — open the Zoom client AND start Nixon recording via the
  // existing auto-start path. Re-check isRecording at click time and no-op the
  // record half if a recording is already running (P1's CptHost auto-detect may
  // also fire when the client appears — handleRecordingToggle also no-ops while
  // recording, so we never double-start).
  const handleJoinAndRecord = useCallback(() => {
    // Spec 0015: the recording adopts this calendar event's identity (title + id).
    void joinAndRecord(
      { id: meeting.id, title: meeting.title, zoomUrl: meeting.zoomUrl, startsAt: meeting.startsAt },
      isRecording,
      handleRecordingToggle,
    );
  }, [meeting.id, meeting.title, meeting.zoomUrl, meeting.startsAt, isRecording, handleRecordingToggle]);

  return (
    <div className="flex items-center gap-3 rounded-lg border border-border bg-card px-4 py-3">
      <div className="flex w-16 flex-shrink-0 flex-col">
        <span className="font-mono text-sm tabular-nums text-foreground">
          {validStart ? formatClockTime(start) : '--:--'}
        </span>
        {validStart && (
          <span className="text-xs text-muted-foreground">{formatRelativeStart(start)}</span>
        )}
      </div>
      <div className="min-w-0 flex-1">
        <div className="truncate text-sm font-semibold text-foreground">{title}</div>
        <div className="truncate text-xs text-muted-foreground">{meeting.calendarName}</div>
      </div>
      {meeting.zoomUrl && (
        <div className="flex flex-shrink-0 items-center gap-2">
          <Button variant="outline" size="sm" className="gap-1.5" onClick={handleJoin}>
            <Video className="h-3.5 w-3.5" />
            Join
          </Button>
          <Button variant="brand" size="sm" className="gap-1.5" onClick={handleJoinAndRecord}>
            <CircleDot className="h-3.5 w-3.5" />
            Join &amp; Record
          </Button>
        </div>
      )}
    </div>
  );
}

export default function UpcomingMeetings() {
  const [status, setStatus] = useState<CalendarAccessStatus | null>(null);
  const [meetings, setMeetings] = useState<UpcomingMeeting[]>([]);
  const [dismissed, setDismissed] = useState(false);

  // Read the sticky "dismissed the connect prompt" flag once on mount.
  useEffect(() => {
    try {
      setDismissed(localStorage.getItem(DISMISS_KEY) === 'true');
    } catch {
      /* localStorage unavailable — treat as not dismissed */
    }
  }, []);

  // Refresh access status + (when authorized) the upcoming list.
  const refresh = useCallback(async () => {
    const s = await getCalendarAccessStatus();
    setStatus(s);
    if (s === 'authorized') {
      setMeetings(await getUpcomingMeetings());
    } else {
      setMeetings([]);
    }
  }, []);

  // Initial load + refresh on focus + light interval.
  useEffect(() => {
    void refresh();
    const onFocus = () => void refresh();
    window.addEventListener('focus', onFocus);
    const interval = window.setInterval(() => void refresh(), REFRESH_INTERVAL_MS);
    return () => {
      window.removeEventListener('focus', onFocus);
      window.clearInterval(interval);
    };
  }, [refresh]);


  const handleDismiss = useCallback(() => {
    setDismissed(true);
    try {
      localStorage.setItem(DISMISS_KEY, 'true');
    } catch {
      /* ignore */
    }
  }, []);

  // Authorized → show the Upcoming section (only if there are meetings).
  if (status === 'authorized') {
    if (meetings.length === 0) return null;
    return (
      <section>
        <h2 className="u-section-label mb-2 px-1">
          Upcoming
        </h2>
        <div className="space-y-2">
          {meetings.map((m) => (
            <MeetingRow key={m.id} meeting={m} />
          ))}
        </div>
      </section>
    );
  }

  // Not authorized → unobtrusive connect prompt, unless the user dismissed it or
  // status hasn't loaded / is restricted (no point prompting on restricted).
  const canPrompt =
    (status === 'notDetermined' || status === 'denied') && !dismissed;
  if (!canPrompt) return null;

  return (
    <ConnectCalendarCard
      title="Connect your calendar to see upcoming meetings"
      onDismiss={handleDismiss}
    />
  );
}
