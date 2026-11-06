'use client';

import { CalendarDays, FileAudio, List, Plus } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { PageHeader } from '@/components/ui/page-header';
import { SegmentedControl, type SegmentedOption } from '@/components/ui/segmented-control';
import { SearchMeetingsButton } from '@/components/CommandPalette/SearchMeetingsButton';
import { ProcessMeetingsButton } from '@/components/DeferredBacklog/ProcessMeetingsButton';
import { useImportDialog } from '@/contexts/ImportDialogContext';
import { isMeetingsView, type MeetingsView } from '@/lib/meetings-view';

const MONTH: SegmentedOption = {
  value: 'month',
  label: 'Month',
  icon: <CalendarDays className="h-3.5 w-3.5" aria-hidden="true" />,
};
const LIST: SegmentedOption = {
  value: 'list',
  label: 'List',
  icon: <List className="h-3.5 w-3.5" aria-hidden="true" />,
};

/**
 * With a calendar: widening left to right, one day → a week → a month → everything.
 * Without one: the old All meetings pair, unchanged (specs/0054 W3).
 */
export function viewOptions(calendarConnected: boolean): SegmentedOption[] {
  return calendarConnected
    ? [{ value: 'agenda', label: 'Agenda' }, { value: 'week', label: 'Week' }, MONTH, LIST]
    : [MONTH, LIST];
}

/**
 * The Meetings page header: the title, a line about what's in view, and one view switch
 * for every view — so switching never moves the control you just used.
 */
export function MeetingsHeader({
  subtitle,
  view,
  calendarConnected,
  onChooseView,
  onAddMeeting,
}: {
  subtitle: string;
  /** The view that's showing (after the no-calendar fallback). */
  view: MeetingsView;
  calendarConnected: boolean;
  onChooseView: (view: MeetingsView) => void;
  onAddMeeting: () => void;
}) {
  const { openImportDialog } = useImportDialog();
  const listLike = view === 'list' || view === 'month';
  return (
    <PageHeader
      title="Meetings"
      subtitle={subtitle}
      actions={
        <>
          <SegmentedControl
            aria-label="Meeting view"
            options={viewOptions(calendarConnected)}
            value={view}
            onChange={(next) => isMeetingsView(next) && onChooseView(next)}
          />
          <SearchMeetingsButton />
          {/* specs/0069 W2 — import sits where an imported recording lands: the List and
              Month views. Muted, not brand — it is a secondary action. */}
          {listLike && (
            <button
              type="button"
              onClick={() => openImportDialog()}
              className="flex items-center gap-[7px] rounded-[3px] border border-border bg-card px-[11px] py-[7px] text-[13px] text-muted-foreground transition-colors hover:bg-accent hover:text-foreground focus:outline-none focus-visible:ring-2 focus-visible:ring-ring"
            >
              <FileAudio className="h-[13px] w-[13px]" aria-hidden="true" />
              <span>Import audio</span>
            </button>
          )}
          <ProcessMeetingsButton />
          {/* A meeting added here lives on the calendar views; without a calendar there is
              no view that would show it until it is recorded. */}
          {calendarConnected && (
            <Button variant="brand" onClick={onAddMeeting} className="gap-2">
              <Plus className="h-[13px] w-[13px]" aria-hidden="true" />
              Add meeting
            </Button>
          )}
        </>
      }
    />
  );
}
