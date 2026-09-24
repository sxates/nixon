'use client';

import { useState, useEffect, useCallback, useMemo } from 'react';
import { motion } from 'framer-motion';
import { useRouter } from 'next/navigation';
import { invoke } from '@tauri-apps/api/core';
import { Mic, Loader2, MoreHorizontal, Trash2, CalendarDays, List, FileAudio } from 'lucide-react';
import { Button } from '@/components/ui/button';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu';
import { DeleteMeetingDialog } from '@/components/MeetingDetails/DeleteMeetingDialog';
import { useSidebar } from '@/components/Sidebar/SidebarProvider';
import { useRecordingState } from '@/contexts/RecordingStateContext';
import { useImportDialog } from '@/contexts/ImportDialogContext';
import { AvatarStack } from '@/components/AvatarStack';
import { attendeeSummaryLabel } from '@/lib/attendees';
import type { AgendaAttendee } from '@/lib/day-agenda';
import { MonthCalendar } from '@/components/Meetings/MonthCalendar';
import { SearchMeetingsButton } from '@/components/CommandPalette/SearchMeetingsButton';
import { avatarColorClass } from '@/lib/avatar-colors';
import { PageHeader } from '@/components/ui/page-header';
import { SegmentedControl } from '@/components/ui/segmented-control';
import { RecordingBadge } from '@/components/RecordingBadge';
import { formatReelTag } from '@/lib/reel-number';

/** Month grid vs. list of rows (specs/0054 W3), as an underlined segmented control. */
const VIEW_MODES = [
  { value: 'month', label: 'Month', icon: <CalendarDays className="h-3.5 w-3.5" aria-hidden="true" /> },
  { value: 'list', label: 'List', icon: <List className="h-3.5 w-3.5" aria-hidden="true" /> },
];

// Shape returned by `api_get_meetings` (same as the Home dashboard uses).
// `durationSeconds`/`gist`/`attendees` may be absent — treat absent as "no value".
interface DashboardMeeting {
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

/** Where the Month/List choice is remembered (specs/0054 W3). */
const VIEW_MODE_KEY = 'nixon.meetings.viewMode';

/** Start-of-day (local time) for a given date. */
function startOfDay(d: Date): Date {
  const r = new Date(d);
  r.setHours(0, 0, 0, 0);
  return r;
}

/** Stable local-day key (YYYY-MM-DD, local time) used to bucket meetings by date. */
function dayKey(d: Date): string {
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
function dayHeading(created: Date, now: Date): { primary: string; secondary: string } {
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
function MeetingRow({
  meeting,
  spineClass,
  isRecordingThis = false,
  onOpen,
  onRequestDelete,
}: {
  meeting: DashboardMeeting;
  spineClass: string;
  /** This meeting is the live recording (specs/0029 WS4.4) — show the Recording marker. */
  isRecordingThis?: boolean;
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
          {isRecordingThis && <RecordingBadge />}
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

export default function AllMeetingsPage() {
  const router = useRouter();
  const { refetchMeetings, activeRecordingMeetingId } = useSidebar();
  const { isRecording } = useRecordingState();
  const { openImportDialog } = useImportDialog();
  const [meetings, setMeetings] = useState<DashboardMeeting[]>([]);
  const [isLoading, setIsLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  // Meeting pending deletion (drives the confirm dialog); null when closed.
  const [meetingToDelete, setMeetingToDelete] = useState<DashboardMeeting | null>(null);

  // specs/0054 W3: Month vs List. Persisted so the choice survives navigation —
  // someone who browses by calendar wants it to still be the calendar next time.
  // Read lazily inside the initializer: localStorage is unavailable during SSR.
  const [viewMode, setViewMode] = useState<'list' | 'month'>(() => {
    // Guarded like the write below: storage is unavailable during SSR and can
    // throw in private-browsing modes. A failed read just means the default view.
    try {
      return window.localStorage?.getItem(VIEW_MODE_KEY) === 'month' ? 'month' : 'list';
    } catch {
      return 'list';
    }
  });

  const chooseViewMode = useCallback((mode: 'list' | 'month') => {
    setViewMode(mode);
    try {
      window.localStorage?.setItem(VIEW_MODE_KEY, mode);
    } catch {
      // Private-mode / quota failures must not break the toggle itself.
    }
  }, []);

  useEffect(() => {
  }, []);

  useEffect(() => {
    let cancelled = false;
    const load = async () => {
      try {
        const result = (await invoke('api_get_meetings', { authToken: null })) as DashboardMeeting[];
        if (!cancelled) setMeetings(Array.isArray(result) ? result : []);
      } catch (err) {
        console.error('Failed to load meetings:', err);
        if (!cancelled) setError('Could not load your meetings.');
      } finally {
        if (!cancelled) setIsLoading(false);
      }
    };
    load();
    return () => {
      cancelled = true;
    };
  }, []);

  const handleOpen = useCallback(
    (id: string) => {
      router.push(`/meeting-details?id=${id}`);
    },
    [router],
  );

  const handleDeleted = useCallback(async () => {
    const deletedId = meetingToDelete?.id;
    if (deletedId) {
      setMeetings((prev) => prev.filter((m) => m.id !== deletedId));
    }
    await refetchMeetings();
  }, [meetingToDelete, refetchMeetings]);

  // Group meetings by calendar day (specs/0038 feedback #5). The backend returns
  // newest-first, so first-seen day order is already newest-day-first and each day's
  // rows stay newest-time-first — no extra sort needed. Undated rows (unparseable
  // timestamp) collect into a trailing "Undated" group.
  const dayGroups = useMemo(() => {
    const now = new Date();
    const order: string[] = [];
    const byKey = new Map<
      string,
      { label: string; dateLabel: string; isToday: boolean; items: DashboardMeeting[] }
    >();
    const undated: DashboardMeeting[] = [];

    for (const m of meetings) {
      const created = new Date(m.createdAt);
      if (Number.isNaN(created.getTime())) {
        undated.push(m);
        continue;
      }
      const key = dayKey(created);
      let group = byKey.get(key);
      if (!group) {
        const heading = dayHeading(created, now);
        group = {
          label: heading.primary,
          dateLabel: heading.secondary,
          isToday: key === dayKey(now),
          items: [],
        };
        byKey.set(key, group);
        order.push(key);
      }
      group.items.push(m);
    }

    const groups = order.map((key) => ({ key, ...byKey.get(key)! }));
    if (undated.length > 0) {
      groups.push({ key: 'undated', label: 'Undated', dateLabel: '', isToday: false, items: undated });
    }
    return groups;
  }, [meetings]);

  const hasMeetings = meetings.length > 0;

  // The list is a column of rows and reads best narrow; the month grid is seven
  // columns and needs the width, or every meeting title truncates.
  const contentWidth = viewMode === 'month' ? 'max-w-[1080px]' : 'max-w-[840px]';

  return (
    <motion.div
      initial={{ opacity: 0, y: 12 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ duration: 0.25, ease: 'easeOut' }}
      className="flex h-page flex-col bg-background"
    >
      <PageHeader
        title="All meetings"
        subtitle={
          hasMeetings
            ? `${meetings.length} recording${meetings.length === 1 ? '' : 's'}`
            : 'Every recording you make shows up here'
        }
        actions={
          <>
            {/* specs/0069 W2 — import lives here, not in the nav: this is the page an
                imported recording lands on. Muted, not brand — it is a secondary action,
                and its old amber made it the loudest thing in the sidebar. */}
            <button
              type="button"
              onClick={() => openImportDialog()}
              className="flex items-center gap-[7px] rounded-[3px] border border-border bg-card px-[11px] py-[7px] text-[13px] text-muted-foreground transition-colors hover:bg-accent hover:text-foreground focus:outline-none focus-visible:ring-2 focus-visible:ring-ring"
            >
              <FileAudio className="h-[13px] w-[13px]" aria-hidden="true" />
              <span>Import audio</span>
            </button>
            {/* Search lives here rather than in the sidebar (specs/0054 W3 follow-up):
                this is the page people are on when they cannot find a meeting. */}
            <SearchMeetingsButton />
            <SegmentedControl
              aria-label="Meeting view"
              options={VIEW_MODES}
              value={viewMode}
              onChange={(next) => chooseViewMode(next as 'month' | 'list')}
            />
          </>
        }
      />

      {/* Month mode owns the remaining height (the grid divides it); list mode scrolls. */}
      <div
        className={
          viewMode === 'month'
            ? 'flex min-h-0 flex-1 flex-col overflow-hidden px-4 min-[900px]:px-7 pb-6'
            : 'flex-1 overflow-y-auto px-4 min-[900px]:px-7 pb-12'
        }
      >
        {isLoading ? (
          <div className="flex h-64 flex-shrink-0 items-center justify-center text-muted-foreground">
            <Loader2 className="mr-2 h-5 w-5 animate-spin" />
            <span className="text-sm">Loading meetings…</span>
          </div>
        ) : error ? (
          <div className="flex h-64 flex-col items-center justify-center text-center">
            <p className="text-sm text-muted-foreground">{error}</p>
          </div>
        ) : (
          <div
            className={`mx-auto w-full ${contentWidth} ${
              viewMode === 'month' ? 'flex min-h-0 flex-1 flex-col' : ''
            }`}
          >
            {viewMode === 'month' ? (
              <MonthCalendar
                onOpenMeeting={handleOpen}
                activeRecordingMeetingId={isRecording ? activeRecordingMeetingId : null}
              />
            ) : hasMeetings ? (
              /* Day cards matching Home's week view (specs/0054 W3 follow-up): the
                 bare label + rows read as one undifferentiated stream once there are
                 hundreds of meetings. */
              <div className="space-y-2.5 pb-2">
                {dayGroups.map((group) => (
                  <div
                    key={group.key}
                    className={`overflow-hidden rounded-[3px] border bg-card ${
                      group.isToday ? 'border-brand/45' : 'border-border'
                    }`}
                  >
                    <div className="flex items-center justify-between gap-3 px-4 py-2">
                      <span className="flex min-w-0 items-baseline gap-2">
                        <span
                          className={`u-section-label ${
                            group.isToday ? 'text-brand' : 'text-foreground'
                          }`}
                        >
                          {group.label}
                        </span>
                        {group.dateLabel && (
                          <span className="u-section-label text-muted-foreground">
                            {group.dateLabel}
                          </span>
                        )}
                      </span>
                      <span className="u-section-label flex-shrink-0">
                        {group.items.length} recording{group.items.length === 1 ? '' : 's'}
                      </span>
                    </div>
                    <div className="border-t border-border/50 [&>*:last-child]:border-b-0">
                      {group.items.map((m) => (
                        <MeetingRow
                          key={m.id}
                          meeting={m}
                          spineClass={avatarColorClass(m.id)}
                          isRecordingThis={isRecording && m.id === activeRecordingMeetingId}
                          onOpen={handleOpen}
                          onRequestDelete={setMeetingToDelete}
                        />
                      ))}
                    </div>
                  </div>
                ))}
              </div>
            ) : (
              <div className="flex h-[40vh] flex-col items-center justify-center text-center">
                <div className="mb-4 flex h-14 w-14 items-center justify-center rounded-full bg-muted">
                  <Mic className="h-6 w-6 text-muted-foreground" />
                </div>
                <h2 className="text-base font-semibold text-foreground">No recordings yet</h2>
                <p className="mt-1 max-w-xs text-sm text-muted-foreground">
                  Start recording and your meetings will show up here.
                </p>
                <Button variant="brand" onClick={() => router.push('/')} className="mt-5 gap-2">
                  <Mic className="h-4 w-4" />
                  Go to Home
                </Button>
              </div>
            )}
          </div>
        )}
      </div>

      {meetingToDelete && (
        <DeleteMeetingDialog
          open={!!meetingToDelete}
          onOpenChange={(open) => {
            if (!open) setMeetingToDelete(null);
          }}
          meetingId={meetingToDelete.id}
          meetingTitle={meetingToDelete.title}
          onDeleted={handleDeleted}
        />
      )}
    </motion.div>
  );
}
