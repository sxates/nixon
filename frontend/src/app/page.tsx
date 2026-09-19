'use client';

/**
 * Home = the Today view (specs/0036 WS7).
 *
 * An 8am–6pm vertical hour-grid timeline of today's calendar events and recordings
 * (from `api_get_day_agenda`), auto-expanded to fit any item — or the current time —
 * outside that window. A "now" line marks the current time; each block is styled by
 * phase (past-recorded / past-unrecorded / happening-now / upcoming) and routed on
 * click (`routeForItem`):
 *   - upcoming calendar occurrence → mint/return its `scheduled` meeting
 *     (`api_ensure_scheduled_meeting`) and open the Prep tab;
 *   - a recorded meeting → its details;
 *   - a happening-now joinable event → the existing Join & Record path (with the
 *     recurring-series key threaded through).
 *
 * The old "Recent recordings" list is gone — history lives in "All meetings"
 * (`/meetings`), reachable from the header.
 *
 * Decomposed (specs/0042 WS5): agenda data/nav/dismiss state lives in
 * `hooks/useDayAgenda`; the header/toolbar/timeline/week sections are components
 * under `components/Today/`. This page is click routing + composition.
 */

import { Suspense, useCallback, useMemo } from 'react';
import { motion } from 'framer-motion';
import { useRouter } from 'next/navigation';
import { invoke } from '@tauri-apps/api/core';
import { Loader2 } from 'lucide-react';
import { toast } from 'sonner';
import { useRecordingState } from '@/contexts/RecordingStateContext';
import { useTranscripts } from '@/contexts/TranscriptContext';
import { useSidebar } from '@/components/Sidebar/SidebarProvider';
import type { DayAgendaItem } from '@/lib/day-agenda';
import { joinAndRecord } from '@/lib/calendar';
import {
  routeForItem,
  localDateKey,
  parseLocalDateKey,
  dayLabel,
  type TimelineContext,
} from '@/lib/today-timeline';
import { useDayAgenda } from '@/hooks/useDayAgenda';
import { TodayHeader } from '@/components/Today/TodayHeader';
import { TodayToolbar } from '@/components/Today/TodayToolbar';
import { DayTimeline } from '@/components/Today/DayTimeline';
import { WeekView } from '@/components/Today/WeekView';
import { ConnectCalendarNudge } from '@/components/Today/ConnectCalendarNudge';

function HomeView() {
  const router = useRouter();
  const { isRecording } = useRecordingState();
  const { handleNewNote, handleRecordingToggle, activeRecordingMeetingId } = useSidebar();
  const { currentMeetingId, meetingTitle } = useTranscripts();

  const {
    items,
    visibleItems,
    calendarStatus,
    loaded,
    now,
    viewDate,
    viewMode,
    viewIsToday,
    weekDays,
    weekItems,
    weekLoaded,
    refresh,
    goToDate,
    goPrev,
    goNext,
    goToday,
    switchMode,
    openDay,
    handleHide,
  } = useDayAgenda();

  const liveTitle = meetingTitle && meetingTitle !== '+ New Call' ? meetingTitle.trim() : null;
  // Which timeline row is the live recording (the backend doesn't know "live").
  const recordingThisId = useMemo(() => {
    if (!isRecording) return null;
    for (const liveId of [activeRecordingMeetingId, currentMeetingId]) {
      if (!liveId) continue;
      const byId = items.find((it) => it.meetingId && it.meetingId === liveId);
      if (byId) return byId.id;
    }
    if (liveTitle) {
      const byTitle = items.find((it) => it.title?.trim().toLowerCase() === liveTitle.toLowerCase());
      if (byTitle) return byTitle.id;
    }
    return null;
  }, [isRecording, activeRecordingMeetingId, currentMeetingId, liveTitle, items]);

  const ctx: TimelineContext = useMemo(
    () => ({ now, isRecording, recordingThisId }),
    [now, isRecording, recordingThisId],
  );

  // Range label for week mode (e.g. "Jul 6 – Jul 12"), reused in the header + summary.
  const weekRangeLabel = useMemo(() => {
    const fmt = (k: string) =>
      parseLocalDateKey(k).toLocaleDateString([], { month: 'short', day: 'numeric' });
    return `${fmt(weekDays[0])} – ${fmt(weekDays[6])}`;
  }, [weekDays]);
  const weekHasToday = weekDays.includes(localDateKey(now));

  // Primary date label shown next to the nav arrows.
  const navLabel = viewMode === 'week' ? (weekHasToday ? 'This week' : weekRangeLabel) : dayLabel(viewDate, now);

  // Summary line under the greeting — reflects the day (or week) in view.
  const daySummary = useMemo(() => {
    if (viewMode === 'week') {
      const total = weekItems.reduce((n, arr) => n + arr.filter((it) => !it.dismissed).length, 0);
      const range = weekHasToday
        ? `Week of ${parseLocalDateKey(weekDays[0]).toLocaleDateString([], { month: 'long', day: 'numeric' })}`
        : weekRangeLabel;
      return total > 0 ? `${range} · ${total} meeting${total === 1 ? '' : 's'}` : range;
    }
    const parts = [
      parseLocalDateKey(viewDate).toLocaleDateString([], {
        weekday: 'long',
        month: 'long',
        day: 'numeric',
      }),
    ];
    const total = visibleItems.length;
    const recorded = visibleItems.filter((it) => it.status.recorded).length;
    if (total > 0) parts.push(`${total} meeting${total === 1 ? '' : 's'}`);
    if (recorded > 0) parts.push(`${recorded} recorded`);
    return parts.join(' · ');
  }, [viewMode, viewDate, visibleItems, weekItems, weekDays, weekRangeLabel, weekHasToday]);

  const handleRecord = useCallback(() => {
    sessionStorage.setItem('autoStartRecording', 'true');
    router.push('/record');
  }, [router]);

  // Explicit Join & Record (the live-meeting button) — opens Zoom + starts recording.
  // Kept separate from the body click so viewing a meeting never auto-joins it.
  const handleJoin = useCallback(
    (item: DayAgendaItem) => {
      void joinAndRecord(
        {
          id: item.id,
          title: item.title,
          zoomUrl: item.zoomUrl,
          startsAt: item.startTime,
          seriesKey: item.seriesKey ?? null,
        },
        isRecording,
        handleRecordingToggle,
      );
    },
    [isRecording, handleRecordingToggle],
  );

  // Click routing: pure decision → side effect (specs/0036). The body click only ever
  // views/opens — Join & Record is the separate `handleJoin` button.
  const handleSelect = useCallback(
    async (item: DayAgendaItem) => {
      const route = routeForItem(item, ctx);
      switch (route.kind) {
        case 'return':
          router.push('/record');
          return;
        case 'open':
          router.push(`/meeting-details?id=${route.meetingId}`);
          return;
        case 'prep':
          try {
            const meetingId = await invoke<string>('api_ensure_scheduled_meeting', {
              calendarEventId: route.calendarEventId,
              seriesKey: route.seriesKey,
              title: route.title,
              occurrenceStart: route.occurrenceStart,
            });
            router.push(`/meeting-details?id=${meetingId}&tab=prep`);
          } catch (err) {
            console.error('Failed to open prep:', err);
            toast.error('Could not open prep for this meeting', {
              description: err instanceof Error ? err.message : String(err),
            });
          }
          return;
        case 'none':
          return;
      }
    },
    [ctx, router],
  );

  return (
    <motion.div
      initial={{ opacity: 0, y: 12 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ duration: 0.25, ease: 'easeOut' }}
      className="flex h-page flex-col bg-background"
    >
      {/* Header — greeting + day summary, ⌘K search, Ask AI, New note, Record. */}
      <TodayHeader
        now={now}
        daySummary={daySummary}
        isRecording={isRecording}
        onNewNote={() => void handleNewNote()}
        onRecord={handleRecord}
      />

      {/* Toolbar — date nav + Day/Week toggle + quick actions; fixed above the scroll region. */}
      <TodayToolbar
        viewMode={viewMode}
        viewDate={viewDate}
        navLabel={navLabel}
        viewIsToday={viewIsToday}
        onPrev={goPrev}
        onNext={goNext}
        onToday={goToday}
        onGoToDate={goToDate}
        onSwitchMode={switchMode}
      />

      {/* Body — the day timeline / week list; scrolls beneath the fixed toolbar. */}
      <div className="flex-1 overflow-y-auto px-7 pb-12">
        <div className="mx-auto max-w-[840px]">
          {(calendarStatus === 'notDetermined' || calendarStatus === 'denied') && (
            <div className="mb-4">
              <ConnectCalendarNudge onConnected={() => void refresh()} />
            </div>
          )}

          {viewMode === 'week' ? (
            !weekLoaded ? (
              <div className="flex h-64 items-center justify-center text-muted-foreground">
                <Loader2 className="mr-2 h-5 w-5 animate-spin" />
                <span className="text-sm">Loading your week…</span>
              </div>
            ) : (
              <WeekView
                weekDays={weekDays}
                weekItems={weekItems}
                ctx={ctx}
                now={now}
                onSelectItem={handleSelect}
                onOpenDay={openDay}
              />
            )
          ) : !loaded ? (
            <div className="flex h-64 items-center justify-center text-muted-foreground">
              <Loader2 className="mr-2 h-5 w-5 animate-spin" />
              <span className="text-sm">Loading your day…</span>
            </div>
          ) : (
            <DayTimeline
              visibleItems={visibleItems}
              now={now}
              viewIsToday={viewIsToday}
              viewDate={viewDate}
              ctx={ctx}
              onSelect={handleSelect}
              onJoin={handleJoin}
              onHide={(it) => void handleHide(it)}
            />
          )}
        </div>
      </div>
    </motion.div>
  );
}

export default function Home() {
  // `useSearchParams` (in useDayAgenda, for the `?day=` restore) requires a Suspense
  // boundary — same pattern as /meeting-details, /ask, /saved-question.
  return (
    <Suspense fallback={<div className="h-page bg-background" />}>
      <HomeView />
    </Suspense>
  );
}
