'use client';

import { CalendarClock, CalendarDays, CalendarRange, FileAudio, List, Plus } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { PageHeader } from '@/components/ui/page-header';
import { SegmentedControl, type SegmentedOption } from '@/components/ui/segmented-control';
import { SearchMeetingsButton } from '@/components/CommandPalette/SearchMeetingsButton';
import { ProcessMeetingsButton } from '@/components/DeferredBacklog/ProcessMeetingsButton';
import { useImportDialog } from '@/contexts/ImportDialogContext';
import { isMeetingsView, MEETINGS_CONTENT_WIDTH, type MeetingsView } from '@/lib/meetings-view';
import { cn } from '@/lib/utils';

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
 * The Meetings page header: the title, a line about what's in view, and the page actions on
 * the right (the Action items / People pattern). The view switch is its own row,
 * `MeetingsViewTabs`, right above the content.
 */
export function MeetingsHeader({
  subtitle,
  calendarConnected,
  onAddMeeting,
}: {
  subtitle: string;
  calendarConnected: boolean;
  onAddMeeting: () => void;
}) {
  const { openImportDialog } = useImportDialog();
  return (
    <PageHeader
      title="Meetings"
      subtitle={subtitle}
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

/**
 * The view switch, in its own row right above the content, like the Action items filter
 * bar: same gutters, centred on the shared Meetings content width.
 */
export function MeetingsViewTabs({
  view,
  calendarConnected,
  onChooseView,
  navSlotRef,
}: {
  view: MeetingsView;
  calendarConnected: boolean;
  onChooseView: (view: MeetingsView) => void;
  /** Receives the right-hand slot that each view's date navigation portals into. */
  navSlotRef?: (el: HTMLDivElement | null) => void;
}) {
  return (
    <div className="flex-shrink-0 px-4 min-[900px]:px-7 pb-3">
      <div
        className={cn(
          'mx-auto flex w-full flex-wrap items-center justify-between gap-x-4 gap-y-2',
          MEETINGS_CONTENT_WIDTH,
        )}
      >
        <SegmentedControl
          aria-label="Meeting view"
          options={viewOptions(calendarConnected)}
          value={view}
          onChange={(next) => isMeetingsView(next) && onChooseView(next)}
        />
        <div ref={navSlotRef} className="flex items-center gap-2" />
      </div>
    </div>
  );
}
