'use client';

import { CalendarClock, CalendarDays, CalendarRange, FileAudio, List, Plus } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { PageHeader } from '@/components/ui/page-header';
import { SegmentedControl, type SegmentedOption } from '@/components/ui/segmented-control';
import { SearchMeetingsButton } from '@/components/CommandPalette/SearchMeetingsButton';
import { ProcessMeetingsButton } from '@/components/DeferredBacklog/ProcessMeetingsButton';
import { useImportDialog } from '@/contexts/ImportDialogContext';
import { isMeetingsView, type MeetingsView } from '@/lib/meetings-view';

// Every option carries an icon at the same size, so the four read as one control.
const AGENDA: SegmentedOption = {
  value: 'agenda',
  label: 'Agenda',
  icon: <CalendarClock className="h-3.5 w-3.5" aria-hidden="true" />,
};
const WEEK: SegmentedOption = {
  value: 'week',
  label: 'Week',
  icon: <CalendarRange className="h-3.5 w-3.5" aria-hidden="true" />,
};
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
    ? [AGENDA, WEEK, MONTH, LIST]
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
  return (
    <PageHeader
      title="Meetings"
      subtitle={subtitle}
      afterTitle={
        <SegmentedControl
          aria-label="Meeting view"
          options={viewOptions(calendarConnected)}
          value={view}
          onChange={(next) => isMeetingsView(next) && onChooseView(next)}
        />
      }
      actions={
        <>
          <SearchMeetingsButton />
          {/* Import audio: an icon button beside Search, on every view (owner, 2026-11-05).
              Same 32px box and 12px glyph as SearchMeetingsButton. */}
          <button
            type="button"
            onClick={() => openImportDialog()}
            aria-label="Import audio"
            title="Import audio"
            className="inline-flex h-8 w-8 flex-shrink-0 items-center justify-center rounded-[3px] border border-border bg-card text-muted-foreground transition-colors hover:bg-accent hover:text-foreground focus:outline-none focus-visible:ring-2 focus-visible:ring-ring"
          >
            <FileAudio className="h-3 w-3" aria-hidden="true" />
          </button>
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
