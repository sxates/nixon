'use client';

/**
 * Home = Meetings: the former Today and All meetings pages as one page.
 *
 * With a calendar connected it offers four views in one switch — Agenda (one day's hour
 * grid, specs/0036 WS7), Week, Month and List (every recording, newest first). Without
 * one it is the All meetings page as it was, Month and List, under the connect-calendar
 * banner. The page remembers the view used last (`useMeetingsView`); `/meetings`
 * redirects here.
 *
 * Composition only: view choice in `hooks/useMeetingsView`, agenda data in
 * `hooks/useDayAgenda`, row actions in `hooks/useAgendaActions`, and each view's body
 * under `components/Meetings/`.
 */

import { Suspense, useCallback, useMemo, useState } from 'react';
import { useRouter } from 'next/navigation';
import { PageLoading } from '@/components/ui/empty-state';
import { useRecordingState } from '@/contexts/RecordingStateContext';
import { useSidebar } from '@/components/Sidebar/SidebarProvider';
import { useDayAgenda } from '@/hooks/useDayAgenda';
import { useAgendaActions } from '@/hooks/useAgendaActions';
import { useCalendarConnected } from '@/hooks/useCalendarConnected';
import { useMeetingsUrlSync, useMeetingsView } from '@/hooks/useMeetingsView';
import { useRecordedMeetings } from '@/hooks/useRecordedMeetings';
import { agendaNavLabel, agendaSummary } from '@/lib/agenda-labels';
import { MEETINGS_CONTENT_WIDTH, needsCalendar } from '@/lib/meetings-view';
import { PAGE_GUTTER } from '@/lib/page-width';
import { cn } from '@/lib/utils';
import { ConnectCalendarNudge } from '@/components/Today/ConnectCalendarNudge';
import { AgendaPane } from '@/components/Meetings/AgendaPane';
import { AgendaDialogs } from '@/components/Meetings/AgendaDialogs';
import { MeetingsHeader, MeetingsViewTabs } from '@/components/Meetings/MeetingsHeader';
import { NavSlotProvider } from '@/components/Meetings/NavSlot';
import { MeetingsList } from '@/components/Meetings/MeetingsList';
import { MonthCalendar } from '@/components/Meetings/MonthCalendar';

function MeetingsView() {
  const router = useRouter();
  const { isRecording } = useRecordingState();
  const { activeRecordingMeetingId } = useSidebar();
  const calendarConnected = useCalendarConnected();
  // The view-tab row's right-hand slot; each view's date navigation portals into it.
  const [navSlot, setNavSlot] = useState<HTMLDivElement | null>(null);
  const { view, shown, chooseView } = useMeetingsView(calendarConnected);
  // Until the calendar check resolves, a calendar view waits on a loader rather than
  // drawing an agenda that may be about to become the List.
  const pendingCalendar = calendarConnected === null && needsCalendar(shown);

  const agendaMode = pendingCalendar ? null : shown === 'agenda' ? 'day' : shown === 'week' ? 'week' : null;
  const agenda = useDayAgenda(agendaMode);
  const rows = useMemo(() => [agenda.items, ...agenda.weekItems], [agenda.items, agenda.weekItems]);
  const actions = useAgendaActions({ now: agenda.now, rows });
  useMeetingsUrlSync(view, shown, agenda.viewDate);

  // The List's rows, and the recording count both List and Month put under the title.
  const recorded = useRecordedMeetings(!agendaMode && !pendingCalendar);

  const openDay = useCallback(
    (key: string) => {
      chooseView('agenda');
      agenda.goToDate(key);
    },
    [chooseView, agenda],
  );

  const labelInput = {
    mode: shown === 'week' ? ('week' as const) : ('day' as const),
    now: agenda.now,
    viewDate: agenda.viewDate,
    weekDays: agenda.weekDays,
    visibleItems: agenda.visibleItems,
    weekItems: agenda.weekItems,
    ctx: actions.ctx,
  };
  const count = recorded.meetings.length;
  const subtitle = agendaMode
    ? agendaSummary(labelInput)
    : count > 0
      ? `${count} recording${count === 1 ? '' : 's'}`
      : 'Every recording you make shows up here';

  return (
    // A short opacity fade, no travel (docs/design-system.md §Motion).
    <div className="flex h-page animate-fade-in flex-col bg-background">
      <MeetingsHeader
        subtitle={subtitle}
        calendarConnected={calendarConnected !== false}
        onAddMeeting={actions.openAdd}
      />

      <AgendaDialogs
        actions={actions}
        defaultDateKey={agenda.viewDate}
        onChanged={() => void agenda.refresh()}
      />

      {/* specs/0069 W4 — gated on `calendarConnected` (EventKit OR Google), never the
          EventKit-only status; `null` (not yet resolved) hides it. */}
      {calendarConnected === false && (
        <div className={cn('flex-shrink-0', PAGE_GUTTER)}>
          <div className={`mx-auto mb-4 w-full ${MEETINGS_CONTENT_WIDTH}`}>
            <ConnectCalendarNudge />
          </div>
        </div>
      )}

      {!pendingCalendar && (
        <MeetingsViewTabs
          view={shown}
          calendarConnected={calendarConnected !== false}
          onChooseView={chooseView}
          navSlotRef={setNavSlot}
        />
      )}

      <NavSlotProvider value={navSlot}>

      {pendingCalendar ? (
        <PageLoading fill="pane" />
      ) : agendaMode ? (
        <AgendaPane
          mode={agendaMode}
          agenda={agenda}
          actions={actions}
          navLabel={agendaNavLabel(labelInput)}
          onOpenDay={openDay}
        />
      ) : shown === 'month' ? (
        // Month owns the remaining height (the grid divides it); the list scrolls. The month
        // grid is seven columns and needs the width, or every title truncates.
        <div className={cn('flex min-h-0 flex-1 flex-col overflow-hidden pb-6', PAGE_GUTTER)}>
          <div className={`mx-auto flex min-h-0 w-full flex-1 flex-col ${MEETINGS_CONTENT_WIDTH}`}>
            <MonthCalendar
              onOpenMeeting={(id) => router.push(`/meeting-details?id=${id}`)}
              activeRecordingMeetingId={isRecording ? activeRecordingMeetingId : null}
              events={
                calendarConnected ? { ctx: actions.ctx, onSelect: actions.handleSelect } : undefined
              }
            />
          </div>
        </div>
      ) : (
        <div className={cn('flex-1 overflow-y-auto pb-12', PAGE_GUTTER)}>
          <div className={`mx-auto w-full ${MEETINGS_CONTENT_WIDTH}`}>
            <MeetingsList
              meetings={recorded.meetings}
              isLoading={recorded.isLoading}
              error={recorded.error}
              onDeleted={recorded.removeMeeting}
            />
          </div>
        </div>
      )}
      </NavSlotProvider>
    </div>
  );
}

export default function Home() {
  // `useSearchParams` (the `?view=` / `?day=` restore) requires a Suspense boundary —
  // same pattern as /meeting-details, /ask, /saved-question.
  return (
    <Suspense fallback={<div className="h-page bg-background" />}>
      <MeetingsView />
    </Suspense>
  );
}
