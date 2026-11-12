'use client';

import { Button } from '@/components/ui/button';
import { DeckIcon } from '@/components/ui/deck-icon';
import { IconButton } from '@/components/ui/icon-button';
import { FileAudio, Plus } from '@/components/ui/icons';
import { PageHeader } from '@/components/ui/page-header';
import { Tabs, TabsList, TabsTrigger } from '@/components/ui/tabs';
import { SearchMeetingsButton } from '@/components/CommandPalette/SearchMeetingsButton';
import { ProcessMeetingsButton } from '@/components/DeferredBacklog/ProcessMeetingsButton';
import { useImportDialog } from '@/contexts/ImportDialogContext';
import { isMeetingsView, MEETINGS_CONTENT_WIDTH, type MeetingsView } from '@/lib/meetings-view';
import { PAGE_GUTTER } from '@/lib/page-width';
import { cn } from '@/lib/utils';

export interface MeetingsViewOption {
  value: MeetingsView;
  label: string;
}

// Text-only view tabs: no per-tab glyphs (docs/design-system.md §Icons).
const AGENDA: MeetingsViewOption = { value: 'agenda', label: 'Agenda' };
const WEEK: MeetingsViewOption = { value: 'week', label: 'Week' };
const MONTH: MeetingsViewOption = { value: 'month', label: 'Month' };
const LIST: MeetingsViewOption = { value: 'list', label: 'List' };

/**
 * With a calendar: widening left to right, one day → a week → a month → everything.
 * Without one: the old All meetings pair, unchanged (specs/0054 W3).
 */
export function viewOptions(calendarConnected: boolean): MeetingsViewOption[] {
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
    // `width="page"`: the title keeps the content's left edge when the rail is collapsed
    // (audit 2026-11-11: "Meetings" at x=92, the cards at x=132).
    <PageHeader
      width="page"
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
  );
}

/**
 * The view switch, in its own row right above the content, like the Action items filter
 * bar: same gutters, centred on the shared Meetings content width.
 *
 * `Tabs`, not a SegmentedControl: Agenda / Week / Month / List change WHICH view is on
 * screen, and tabs switch the view (docs/design-system.md). The row owns the hairline so
 * the date navigation on the right sits on the same rule as the tabs; the tablist itself
 * draws none. There is no `TabsContent` — the panes are rendered by the page — so the
 * triggers drop Radix's `aria-controls`, which would point at panels that don't exist.
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
      <Tabs
        value={view}
        onValueChange={(next) => isMeetingsView(next) && onChooseView(next)}
        className={cn(
          'mx-auto flex w-full flex-wrap items-end justify-between gap-x-4 border-b border-border',
          MEETINGS_CONTENT_WIDTH,
        )}
      >
        <TabsList aria-label="Meeting view" className="border-b-0">
          {viewOptions(calendarConnected).map((option) => (
            <TabsTrigger key={option.value} value={option.value} aria-controls={undefined}>
              {option.label}
            </TabsTrigger>
          ))}
        </TabsList>
        <div ref={navSlotRef} className="flex items-center gap-2 pb-1.5" />
      </Tabs>
    </div>
  );
}
