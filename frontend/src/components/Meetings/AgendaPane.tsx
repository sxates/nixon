'use client';

import { Loader2 } from 'lucide-react';
import type { useDayAgenda } from '@/hooks/useDayAgenda';
import type { AgendaActions } from '@/hooks/useAgendaActions';
import { TodayToolbar } from '@/components/Today/TodayToolbar';
import { DayTimeline } from '@/components/Today/DayTimeline';
import { WeekView } from '@/components/Today/WeekView';
import { GoogleReconnectRow } from '@/components/Today/GoogleReconnectRow';
import { MEETINGS_CONTENT_WIDTH } from '@/lib/meetings-view';

type Agenda = ReturnType<typeof useDayAgenda>;

function Loading({ label }: { label: string }) {
  return (
    <div className="flex h-64 items-center justify-center text-muted-foreground">
      <Loader2 className="mr-2 h-5 w-5 animate-spin" />
      <span className="text-sm">{label}</span>
    </div>
  );
}

/**
 * The Meetings page's Agenda and Week views (formerly the Today page's body): the fixed
 * date toolbar, then the day timeline or the week's day cards in a scroll region beneath
 * it (specs/0041 WS6).
 */
export function AgendaPane({
  mode,
  agenda,
  actions,
  navLabel,
  onOpenDay,
}: {
  mode: 'day' | 'week';
  agenda: Agenda;
  actions: AgendaActions;
  navLabel: string;
  /** Week → day drill-in: open that day in the Agenda view. */
  onOpenDay: (key: string) => void;
}) {
  const { ctx, handleSelect, handleJoin, handleEditManual, handleDeleteManual, handleRecordManual } =
    actions;
  const onHide = (it: Parameters<Agenda['handleHide']>[0]) => void agenda.handleHide(it);

  return (
    <>
      <TodayToolbar
        viewMode={mode}
        viewDate={agenda.viewDate}
        navLabel={navLabel}
        viewIsToday={agenda.viewIsToday}
        onPrev={agenda.goPrev}
        onNext={agenda.goNext}
        onToday={agenda.goToday}
        onGoToDate={agenda.goToDate}
      />

      <div className="flex-1 overflow-y-auto px-4 min-[900px]:px-7 pb-12">
        <div className={`mx-auto ${MEETINGS_CONTENT_WIDTH}`}>
          {/* specs/0074 W2 — Google connected but its grant lapsed: nothing syncs until
              reconnect. Renders nothing otherwise, so its bottom margin lives on the row. */}
          <GoogleReconnectRow />

          {mode === 'week' ? (
            !agenda.weekLoaded ? (
              <Loading label="Loading your week…" />
            ) : (
              <WeekView
                weekDays={agenda.weekDays}
                weekItems={agenda.weekItems}
                ctx={ctx}
                now={agenda.now}
                onSelectItem={handleSelect}
                onOpenDay={onOpenDay}
                actions={{
                  onHide,
                  onEdit: handleEditManual,
                  onDelete: handleDeleteManual,
                  onRecord: handleRecordManual,
                }}
              />
            )
          ) : !agenda.loaded ? (
            <Loading label="Loading your day…" />
          ) : (
            <DayTimeline
              visibleItems={agenda.visibleItems}
              now={agenda.now}
              viewIsToday={agenda.viewIsToday}
              viewDate={agenda.viewDate}
              ctx={ctx}
              onSelect={handleSelect}
              onJoin={handleJoin}
              onHide={onHide}
              onEdit={handleEditManual}
              onDelete={handleDeleteManual}
              onRecord={handleRecordManual}
            />
          )}
        </div>
      </div>
    </>
  );
}
