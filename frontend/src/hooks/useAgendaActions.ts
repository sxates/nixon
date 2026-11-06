import { useCallback, useMemo, useState } from 'react';
import { useRouter } from 'next/navigation';
import { toast } from 'sonner';
import { useRecordingState } from '@/contexts/RecordingStateContext';
import { useTranscripts } from '@/contexts/TranscriptContext';
import { useSidebar } from '@/components/Sidebar/SidebarProvider';
import { useProcessingMeetingIds } from '@/contexts/ProcessingMeetingsContext';
import type { DayAgendaItem } from '@/lib/day-agenda';
import { joinAndRecord, resolveRecordingStartsAt } from '@/lib/calendar';
import { prepRouteForEvent } from '@/lib/prep';
import {
  routeForItem,
  openMeetingUrl,
  findRecordingRowId,
  canEditManualItem,
  type TimelineContext,
} from '@/lib/today-timeline';

/**
 * What an agenda row can do — shared by the Agenda, Week and Month views (moved out of the
 * old Today page when it merged into Meetings). Click routing is a pure decision → side
 * effect (specs/0036): the body click only ever views/opens; Join & Record is its own
 * button (`handleJoin`), so viewing a meeting never auto-joins it.
 */
export function useAgendaActions({
  now,
  rows,
}: {
  now: Date;
  /** Every agenda row on screen, so the live recording's row can be found among them. */
  rows: DayAgendaItem[][];
}) {
  const router = useRouter();
  const { isRecording } = useRecordingState();
  const { handleRecordingToggle, activeRecordingMeetingId } = useSidebar();
  const { currentMeetingId, meetingTitle } = useTranscripts();

  const [addOpen, setAddOpen] = useState(false);
  // specs/0069 W3 — editing/deleting a manually added entry reuses AddMeetingDialog's
  // `editing` mode and a dedicated confirm dialog, both driven by the target item.
  const [editingItem, setEditingItem] = useState<DayAgendaItem | null>(null);
  const [deletingItem, setDeletingItem] = useState<DayAgendaItem | null>(null);
  // A RECORDED meeting deleted from its row — the list's dialog, which also removes the
  // recording on disk. Manual entries keep their own dialog above.
  const [deletingRecorded, setDeletingRecorded] = useState<DayAgendaItem | null>(null);

  const liveTitle = meetingTitle && meetingTitle !== '+ New Call' ? meetingTitle.trim() : null;
  // Which row is the live recording (the backend doesn't know "live").
  const recordingThisId = useMemo(
    () =>
      findRecordingRowId(rows, {
        isRecording,
        liveIds: [activeRecordingMeetingId, currentMeetingId],
        liveTitle,
      }),
    [isRecording, activeRecordingMeetingId, currentMeetingId, liveTitle, rows],
  );

  // Meetings with work in flight — the rail's own sources, so the agenda agrees with it.
  const processingIds = useProcessingMeetingIds();
  const ctx: TimelineContext = useMemo(
    () => ({ now, isRecording, recordingThisId, processingIds }),
    [now, isRecording, recordingThisId, processingIds],
  );

  // Explicit Join & Record (the live-meeting button) — opens Zoom + starts recording.
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
  // `startsAt` (fix round 2, specs/0069 followup a): pressing Record early must NOT thread
  // the future scheduled start through — see `resolveRecordingStartsAt`'s doc comment.
  // Shared with the meeting-details page's manual Record button (specs/0069b followup) so
  // the now-vs-scheduled decision has exactly one implementation.
  const handleRecordManual = useCallback(
    (item: DayAgendaItem) => {
      const calendarEventId = item.calendarEventId;
      if (!calendarEventId) {
        console.error(
          '[meetings] manual item missing calendarEventId; refusing to record to avoid a duplicate meeting',
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

  const openAdd = useCallback(() => setAddOpen(true), []);
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

  return {
    ctx,
    addOpen,
    openAdd,
    editingItem,
    deletingItem,
    setDeletingItem,
    deletingRecorded,
    setDeletingRecorded,
    closeManualDialog,
    handleJoin,
    handleRecordManual,
    handleEditManual,
    handleDeleteManual,
    handleSelect,
  };
}

export type AgendaActions = ReturnType<typeof useAgendaActions>;
