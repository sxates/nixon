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

import { Suspense, useCallback, useMemo, useState } from 'react';
import { motion } from 'framer-motion';
import { useRouter } from 'next/navigation';
import { Loader2 } from 'lucide-react';
import { toast } from 'sonner';
import { useRecordingState } from '@/contexts/RecordingStateContext';
import { useTranscripts } from '@/contexts/TranscriptContext';
import { useSidebar } from '@/components/Sidebar/SidebarProvider';
import type { DayAgendaItem } from '@/lib/day-agenda';
import { joinAndRecord, resolveRecordingStartsAt } from '@/lib/calendar';
import { DeleteMeetingDialog } from '@/components/MeetingDetails/DeleteMeetingDialog';
import {
  routeForItem,
  openMeetingUrl,
  localDateKey,
  parseLocalDateKey,
  dayLabel,
  findRecordingRowId,
  canEditManualItem,
  type TimelineContext,
} from '@/lib/today-timeline';
import { prepRouteForEvent } from '@/lib/prep';
import { useDayAgenda } from '@/hooks/useDayAgenda';
import { useProcessingMeetingIds } from '@/contexts/ProcessingMeetingsContext';
import { TodayHeader } from '@/components/Today/TodayHeader';
import { TodayToolbar } from '@/components/Today/TodayToolbar';
import { DayTimeline } from '@/components/Today/DayTimeline';
import { DayList } from '@/components/Today/DayList';
import { WeekView } from '@/components/Today/WeekView';
import { ConnectCalendarNudge } from '@/components/Today/ConnectCalendarNudge';
import { GoogleReconnectRow } from '@/components/Today/GoogleReconnectRow';
import { AddMeetingDialog } from '@/components/Today/AddMeetingDialog';
import { DeleteManualMeetingDialog } from '@/components/Today/DeleteManualMeetingDialog';

function HomeView() {
  const router = useRouter();
  const { isRecording } = useRecordingState();
  const { handleRecordingToggle, activeRecordingMeetingId } = useSidebar();
  const { currentMeetingId, meetingTitle } = useTranscripts();
  const [addOpen, setAddOpen] = useState(false);
  // specs/0069 W3 — editing/deleting a manually added entry reuses AddMeetingDialog's
  // `editing` mode and a dedicated confirm dialog, both driven by the target item.
  const [editingItem, setEditingItem] = useState<DayAgendaItem | null>(null);
  const [deletingItem, setDeletingItem] = useState<DayAgendaItem | null>(null);
  // A RECORDED meeting deleted from its row — All Meetings' dialog, which also removes the
  // recording on disk. Manual entries keep their own dialog above.
  const [deletingRecorded, setDeletingRecorded] = useState<DayAgendaItem | null>(null);

  const {
    items,
    visibleItems,
    calendarConnected,
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
  const recordingThisId = useMemo(
    () =>
      findRecordingRowId([items, ...weekItems], {
        isRecording,
        liveIds: [activeRecordingMeetingId, currentMeetingId],
        liveTitle,
      }),
    [isRecording, activeRecordingMeetingId, currentMeetingId, liveTitle, items, weekItems],
  );

  // Meetings with work in flight — the rail's own sources, so the agenda agrees with it.
  const processingIds = useProcessingMeetingIds();
  const ctx: TimelineContext = useMemo(
    () => ({ now, isRecording, recordingThisId, processingIds }),
    [now, isRecording, recordingThisId, processingIds],
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

  // Recording a manual entry goes through the SAME path as Join & Record:
  // `joinAndRecord` creates the meeting with this entry's EVENT id (`calendarEventId`,
  // `nixon-manual:{uuid}`), which makes `api_create_meeting` adopt the scheduled row and
  // promote it (specs/0036), so the prep notes and cached brief carry into the recording
  // instead of a second, empty meeting appearing beside it. The manual item's own `id`
  // IS its meeting id (not its event id) — passing that instead would match nothing and
  // silently create that second meeting, so this refuses rather than guess.
  //
  // `startsAt` (fix round 2, specs/0069 followup a): `canRecordManualItem` no longer
  // requires the entry's scheduled start to have arrived, so pressing Record early must
  // NOT thread the future scheduled start through — see `resolveRecordingStartsAt`'s
  // doc comment for why. Shared with the meeting-details page's manual Record button
  // (specs/0069b followup) so the now-vs-scheduled decision has exactly one implementation.
  const handleRecordManual = useCallback(
    (item: DayAgendaItem) => {
      const calendarEventId = item.calendarEventId;
      if (!calendarEventId) {
        console.error(
          '[today] manual item missing calendarEventId; refusing to record to avoid a duplicate meeting',
          item,
        );
        toast.error('Could not start recording for this meeting');
        return;
      }
      const startsAt = resolveRecordingStartsAt(item.startTime, now);
      void joinAndRecord(
        {
          id: calendarEventId,
          title: item.title,
          zoomUrl: item.zoomUrl,
          startsAt,
          seriesKey: null,
        },
        isRecording,
        handleRecordingToggle,
      );
    },
    [isRecording, handleRecordingToggle, now],
  );

  const handleEditManual = useCallback((item: DayAgendaItem) => setEditingItem(item), []);
  const handleDeleteManual = useCallback(
    (item: DayAgendaItem) =>
      canEditManualItem(item) ? setDeletingItem(item) : setDeletingRecorded(item),
    [],
  );

  // Add and Edit share one dialog instance (AddMeetingDialog's `editing` prop); closing
  // either clears both so the next open starts clean.
  const closeManualDialog = useCallback((open: boolean) => {
    if (!open) {
      setAddOpen(false);
      setEditingItem(null);
    }
  }, []);

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
          router.push(openMeetingUrl(route.meetingId, route.tab));
          return;
        case 'prep':
          try {
            router.push(
              await prepRouteForEvent({
                id: route.calendarEventId,
                title: route.title,
                startsAt: route.occurrenceStart,
                externalId: route.seriesKey,
              }),
            );
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
      {/* Header — greeting + day summary, ⌘K search, Ask AI, Add meeting. */}
      <TodayHeader now={now} daySummary={daySummary} onAddMeeting={() => setAddOpen(true)} />

      <AddMeetingDialog
        open={addOpen || !!editingItem}
        onOpenChange={closeManualDialog}
        defaultDateKey={viewDate}
        editing={
          editingItem
            ? {
                meetingId: editingItem.meetingId ?? editingItem.id,
                title: editingItem.title,
                startsAt: editingItem.startTime,
                endsAt: editingItem.endTime,
                joinUrl: editingItem.zoomUrl,
              }
            : undefined
        }
        // specs/0069b followup — a fresh create carries its new id (see AddMeetingDialog's
        // `onSaved`) and jumps straight to that meeting's Prep tab, the same place clicking
        // it on the timeline would land (`routeForItem`'s manual-entry branch). Editing never
        // carries an id, so it stays on Today. The create path skips `refresh()`: we're
        // navigating away and unmounting this tree, so refreshing Today's now-stale agenda
        // would be wasted work racing the navigation (and risks setting state after unmount).
        onSaved={(createdMeetingId) => {
          if (createdMeetingId) {
            router.push(openMeetingUrl(createdMeetingId, 'prep'));
            return;
          }
          void refresh();
        }}
      />

      <DeleteManualMeetingDialog
        open={!!deletingItem}
        onOpenChange={(open) => {
          if (!open) setDeletingItem(null);
        }}
        meetingId={deletingItem?.meetingId ?? deletingItem?.id ?? null}
        meetingTitle={deletingItem?.title}
        onDeleted={() => {
          setDeletingItem(null);
          void refresh();
        }}
      />

      {deletingRecorded?.meetingId && (
        <DeleteMeetingDialog
          open
          onOpenChange={(open) => {
            if (!open) setDeletingRecorded(null);
          }}
          meetingId={deletingRecorded.meetingId}
          meetingTitle={deletingRecorded.title}
          onDeleted={() => {
            setDeletingRecorded(null);
            void refresh();
          }}
        />
      )}

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
      <div className="flex-1 overflow-y-auto px-4 min-[900px]:px-7 pb-12">
        <div className="mx-auto max-w-[840px]">
          {/* specs/0069 W4 — gated on `calendarConnected` (EventKit OR Google), not the
              EventKit-only `calendarStatus`: a Google-connected user must never be told
              forever to connect a calendar they already connected. `null` (not yet
              resolved) hides it, same as the prior status-based gate did before its
              first read. */}
          {calendarConnected === false && (
            <div className="mb-4">
              <ConnectCalendarNudge />
            </div>
          )}
          {/* specs/0074 W2 — Google connected but its grant lapsed: nothing syncs until
              reconnect. Renders nothing otherwise, so its bottom margin lives on the row itself. */}
          <GoogleReconnectRow />

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
                actions={{
                  onHide: (it) => void handleHide(it),
                  onEdit: handleEditManual,
                  onDelete: handleDeleteManual,
                  onRecord: handleRecordManual,
                }}
              />
            )
          ) : !loaded ? (
            <div className="flex h-64 items-center justify-center text-muted-foreground">
              <Loader2 className="mr-2 h-5 w-5 animate-spin" />
              <span className="text-sm">Loading your day…</span>
            </div>
          ) : viewMode === 'list' ? (
            // specs/0069 W4 — same single-day data + handlers as the grid, presented as
            // a list (the default with no calendar connected).
            <DayList
              items={visibleItems}
              ctx={ctx}
              now={now}
              onSelect={handleSelect}
              onJoin={handleJoin}
              onRecord={handleRecordManual}
              onHide={(it) => void handleHide(it)}
              onEdit={handleEditManual}
              onDelete={handleDeleteManual}
              onAddMeeting={() => setAddOpen(true)}
            />
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
              onEdit={handleEditManual}
              onDelete={handleDeleteManual}
              onRecord={handleRecordManual}
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
