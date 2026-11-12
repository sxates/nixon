'use client';

import { Trash2 } from '@/components/ui/icons';
import { ListRow } from '@/components/ui/list-row';
import { MenuButton } from '@/components/ui/menu-button';
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
import { cn } from '@/lib/utils';

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
 * One line of the tape log (specs/0057 Task 6), on `ListRow`: reel tag and a 3px colour
 * spine in the lead, then the title, the bounded attendee preview (specs/0038 WS8.a —
 * avatars + "Sarah, +2", owner excluded), the duration and the start time as fixed-width
 * columns. The attendee cluster is omitted when the meeting has no roster (never
 * fabricated); the reel cell is blank for a meeting the backend has not numbered.
 *
 * The columns ride inside the row's button (the title slot), so the whole line opens the
 * meeting as it always did; only the `…` menu sits outside it. The attendee column has a
 * real width (audit 2026-11-11: an auto track capped at 38% clipped "Tomas, +1" to "T..").
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
  // 0.1.0 owner feedback: plain text, not the rail's counter — easier to read in a list,
  // and the counter is the live recording's instrument, not a duration.
  const duration = formatDuration(meeting.durationSeconds);
  const title = meeting.title?.trim() || 'Untitled meeting';
  const reelTag = formatReelTag(meeting.reelNumber);
  // Bounded attendee preview (WS8.a) with owner excluded display-only (WS8.b).
  const attendees = meeting.attendees ?? [];
  const attendeePreview = attendeeSummaryLabel(attendees, meeting.attendeeCount ?? 0, 1);
  const column = 'u-row-meta flex-none font-normal tabular-nums';

  return (
    <ListRow
      ruled
      menuReveal="hover"
      onClick={() => onOpen(meeting.id)}
      lead={
        <span className="flex items-center gap-3">
          <span className="w-12 truncate font-mono text-caption text-engrave">{reelTag}</span>
          <span className={cn('h-4 w-[3px] flex-none', spineClass)} aria-hidden="true" />
        </span>
      }
      title={
        <span className="flex items-center gap-3">
          <span className="min-w-0 flex-1 truncate">{title}</span>
          {isRecordingThis ? <RecordingBadge /> : isProcessing && <ProcessingBadge />}
          {/* Hidden below `sm`, where the title needs the width. */}
          <span data-col="attendees" className="hidden w-[200px] flex-none items-center gap-1.5 font-normal sm:flex">
            {attendeePreview && (
              <>
                <AvatarStack attendees={attendees} size="sm" max={3} />
                <span className="u-row-meta min-w-0 truncate">{attendeePreview}</span>
              </>
            )}
          </span>
          <span data-col="duration" className={cn(column, 'w-14 text-right')}>{duration ?? ''}</span>
          <span data-col="time" className={cn(column, 'w-16 text-right')}>
            {validTime ? formatStartTime(created) : ''}
          </span>
        </span>
      }
      menu={
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <MenuButton label="Meeting options" />
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end">
            <DropdownMenuItem
              icon={Trash2}
              variant="destructive"
              onSelect={() => onRequestDelete(meeting)}
            >
              Delete meeting
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
      }
    />
  );
}
