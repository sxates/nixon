'use client';

import { Button } from '@/components/ui/button';
import { DeckIcon } from '@/components/ui/deck-icon';
import { IconButton } from '@/components/ui/icon-button';
import { FileAudio, Plus } from '@/components/ui/icons';
import { PageHeader } from '@/components/ui/page-header';
import { SegmentedControl, type SegmentedOption } from '@/components/ui/segmented-control';
import { SearchMeetingsButton } from '@/components/CommandPalette/SearchMeetingsButton';
import { ProcessMeetingsButton } from '@/components/DeferredBacklog/ProcessMeetingsButton';
import { useImportDialog } from '@/contexts/ImportDialogContext';
import { isMeetingsView, MEETINGS_CONTENT_WIDTH, type MeetingsView } from '@/lib/meetings-view';
import { PAGE_GUTTER } from '@/lib/page-width';
import { cn } from '@/lib/utils';

// Text-only view tabs: no per-tab glyphs (docs/design-system.md §Icons).
const AGENDA: SegmentedOption = { value: 'agenda', label: 'Agenda' };
const WEEK: SegmentedOption = { value: 'week', label: 'Week' };
const MONTH: SegmentedOption = { value: 'month', label: 'Month' };
const LIST: SegmentedOption = { value: 'list', label: 'List' };

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
    // The header sits in the same centred column as the content, so with the rail
    // collapsed (a wider page) the title keeps the content's left edge (audit 2026-11-11:
    // "Meetings" at x=92, the cards at x=132). The gutter moves out to the wrapper.
    <div className={cn('flex-shrink-0', PAGE_GUTTER)}>
      <div className={cn('mx-auto w-full', MEETINGS_CONTENT_WIDTH)}>
        <PageHeader
          className="px-0 min-[900px]:px-0"
          title="Meetings"
          subtitle={subtitle}
          actions={
            <>
              <SearchMeetingsButton />
              {/* Import audio: an icon button beside Search, on every view (owner, 2026-11-05). */}
              <IconButton
                size="md"
                label="Import audio"
                onClick={() => openImportDialog()}
                icon={<DeckIcon icon={FileAudio} size={14} />}
              />
              <ProcessMeetingsButton />
              {/* A meeting added here lives on the calendar views; without a calendar there is
                  no view that would show it until it is recorded. */}
              {calendarConnected && (
                <Button variant="brand" onClick={onAddMeeting}>
                  <Plus aria-hidden="true" />
                  Add meeting
                </Button>
              )}
            </>
          }
        />
      </div>
    </div>
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
    <div className={cn('flex-shrink-0 pb-3', PAGE_GUTTER)}>
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
