'use client';

import { MoreHorizontal, Trash2 } from 'lucide-react';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu';
import { AvatarStack } from '@/components/AvatarStack';
import { attendeeSummaryLabel } from '@/lib/attendees';
import type { AgendaAttendee } from '@/lib/day-agenda';
import { RecordingBadge } from '@/components/RecordingBadge';
import { ProcessingBadge } from '@/components/ProcessingBadge';
import { formatReelTag } from '@/lib/reel-number';

/**
 * One row of the Meetings List view (the former All meetings page, specs/0057 Task 6),
 * plus the day-bucketing helpers the list groups by.
 */

// Shape returned by `api_get_meetings` (same as the Home dashboard uses).
// `durationSeconds`/`gist`/`attendees` may be absent — treat absent as "no value".
export interface DashboardMeeting {
  id: string;
  title: string;
  createdAt: string; // ISO-8601 UTC
  updatedAt?: string;
  durationSeconds?: number;
  gist?: string;
  /**
   * Bounded attendee preview (specs/0038 WS8.a). Owner-inclusive + flagged; the display
   * filter (owner exclusion) lives in `@/lib/attendees`. Absent when the meeting has no
   * roster. Shaped like the Day Agenda's `AgendaAttendee` so `AvatarStack` is reused.
   */
  attendees?: AgendaAttendee[];
  /** Total roster size (owner-inclusive) for the "+N" overflow; absent/0 when no roster. */
  attendeeCount?: number;
  /** Archival reel ordinal (specs/0057): 1-based, oldest first. Absent on legacy DTOs. */
  reelNumber?: number;
}

/** Start-of-day (local time) for a given date. */
function startOfDay(d: Date): Date {
  const r = new Date(d);
  r.setHours(0, 0, 0, 0);
  return r;
}

/** Stable local-day key (YYYY-MM-DD, local time) used to bucket meetings by date. */
export function dayKey(d: Date): string {
  const y = d.getFullYear();
  const m = String(d.getMonth() + 1).padStart(2, '0');
  const day = String(d.getDate()).padStart(2, '0');
  return `${y}-${m}-${day}`;
}

/**
 * Two-part header for a day bucket, mirroring Home's week view: a bold primary
 * ("Today" / "Yesterday" / "Wednesday") and a muted date ("Aug 26"), with the year
 * appended once it differs from the current year.
 *
 * Both parts always render — specs/0038 feedback #5 was that a group could show no
 * date at all, so "Today" alone is not enough.
 */
export function dayHeading(created: Date, now: Date): { primary: string; secondary: string } {
  const dayMs = 24 * 60 * 60 * 1000;
  const diffDays = Math.round(
    (startOfDay(now).getTime() - startOfDay(created).getTime()) / dayMs,
  );
  const sameYear = created.getFullYear() === now.getFullYear();
  const secondary = created.toLocaleDateString([], {
    month: 'short',
    day: 'numeric',
    ...(sameYear ? {} : { year: 'numeric' }),
  });
  if (diffDays === 0) return { primary: 'Today', secondary };
  if (diffDays === 1) return { primary: 'Yesterday', secondary };
  return { primary: created.toLocaleDateString([], { weekday: 'long' }), secondary };
}

/** "8:30 AM" — local start time, matching Home's 12-hour clock. */
function formatStartTime(d: Date): string {
  return d.toLocaleTimeString([], { hour: 'numeric', minute: '2-digit', hour12: true });
}

/**
 * Plain human duration ("42 min", "1h 05m") shown on the row. 0.1.0 owner feedback: this
 * replaced a decorative `TapeCounter` per row — a counter is the live recording's
 * instrument; in a list, plain text reads faster.
 */
function formatDuration(seconds?: number | null): string | null {
  if (seconds == null || !Number.isFinite(seconds) || seconds <= 0) return null;
  const totalMinutes = Math.round(seconds / 60);
  if (totalMinutes < 60) return `${totalMinutes} min`;
  const hours = Math.floor(totalMinutes / 60);
  const minutes = totalMinutes % 60;
  return `${hours}h ${String(minutes).padStart(2, '0')}m`;
}

/**
 * One line of the tape log (specs/0057 Task 6): a hairline-ruled grid — reel tag, a 3px
 * colour spine + title, the bounded attendee preview (specs/0038 WS8.a — avatars +
 * "Sarah, +2", owner excluded), the duration on a counter, and the start time. The
 * attendee cluster is omitted when the meeting has no roster (never fabricated); the
 * reel cell is blank for a meeting the backend has not numbered.
 */
export function MeetingRow({
  meeting,
  spineClass,
  isRecordingThis = false,
  isProcessing = false,
  onOpen,
  onRequestDelete,
}: {
  meeting: DashboardMeeting;
  spineClass: string;
  /** This meeting is the live recording (specs/0029 WS4.4) — show the Recording marker. */
  isRecordingThis?: boolean;
  /** Work is in flight for it (`useProcessingMeetingIds`); Recording wins over it. */
  isProcessing?: boolean;
  onOpen: (id: string) => void;
  onRequestDelete: (meeting: DashboardMeeting) => void;
}) {
  const created = new Date(meeting.createdAt);
  const validTime = !Number.isNaN(created.getTime());
  const seconds = meeting.durationSeconds;
  const hasDuration = seconds != null && Number.isFinite(seconds) && seconds > 0;
  const duration = formatDuration(seconds);
  const title = meeting.title?.trim() || 'Untitled meeting';
  const reelTag = formatReelTag(meeting.reelNumber);
  // Bounded attendee preview (WS8.a) with owner excluded display-only (WS8.b).
  const attendees = meeting.attendees ?? [];
  const attendeePreview = attendeeSummaryLabel(attendees, meeting.attendeeCount ?? 0, 1);

  return (
    <div className="group flex w-full items-center border-b border-border/60 transition-colors hover:bg-accent">
      <button
        type="button"
        onClick={() => onOpen(meeting.id)}
        className="grid min-w-0 flex-1 grid-cols-[64px_1fr_auto_64px_64px] items-center gap-3 px-4 py-2 text-left focus:outline-none focus-visible:ring-2 focus-visible:ring-ring"
      >
        <span className="truncate font-mono text-[11px] text-engrave">{reelTag}</span>
        <span className="flex min-w-0 items-center gap-2.5">
          <span className={`h-4 w-[3px] flex-shrink-0 ${spineClass}`} aria-hidden="true" />
          <span className="min-w-0 flex-1 truncate text-[13.5px] font-semibold text-foreground">
            {title}
          </span>
          {isRecordingThis ? <RecordingBadge /> : isProcessing && <ProcessingBadge />}
        </span>
        {/* fix round 1: the wrapper stays a permanent grid item (5 tracks below `sm`
            too) — only its CONTENTS collapse via `sm:contents`, so `display:none`
            never removes a grid item and shifts the row's later tracks. */}
        <span className="flex max-w-[38%] min-w-0 flex-shrink-0 items-center gap-1.5">
          {attendeePreview && (
            <span className="hidden min-w-0 items-center gap-1.5 sm:contents">
              <AvatarStack attendees={attendees} size="sm" max={3} />
              <span className="truncate text-xs text-muted-foreground">{attendeePreview}</span>
            </span>
          )}
        </span>
        <span className="flex justify-end">
          {hasDuration && (
            // 0.1.0 owner feedback: plain text, not the rail's counter — easier to read in
            // a list, and the counter is the live recording's instrument, not a duration.
            <span className="text-xs tabular-nums text-muted-foreground">{duration}</span>
          )}
        </span>
        <span className="text-right text-xs text-muted-foreground">
          {validTime ? formatStartTime(created) : ''}
        </span>
      </button>
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <button
            type="button"
            aria-label="Meeting options"
            title="Meeting options"
            className="mr-2 inline-flex h-7 w-7 flex-shrink-0 items-center justify-center rounded-md text-muted-foreground opacity-0 transition-opacity hover:bg-background hover:text-foreground focus:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:opacity-100 group-hover:opacity-100 data-[state=open]:opacity-100"
          >
            <MoreHorizontal className="h-4 w-4" />
          </button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end">
          <DropdownMenuItem
            onSelect={() => onRequestDelete(meeting)}
            className="text-destructive focus:text-destructive"
          >
            <Trash2 className="mr-2 h-4 w-4" />
            Delete meeting
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>
    </div>
  );
}
