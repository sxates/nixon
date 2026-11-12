'use client';

import { DateNavigator } from '@/components/ui/date-navigator';
import { DeckIcon } from '@/components/ui/deck-icon';
import { CalendarDays } from '@/components/ui/icons';
import { Input } from '@/components/ui/input';
import { PAGE_GUTTER } from '@/lib/page-width';
import { cn } from '@/lib/utils';
import type { AgendaMode } from '@/hooks/useDayAgenda';
import { MEETINGS_CONTENT_WIDTH } from '@/lib/meetings-view';
import { NavSlotPortal, useHasNavSlot } from '@/components/Meetings/NavSlot';

interface TodayToolbarProps {
  viewMode: AgendaMode;
  viewDate: string;
  /** Primary date label shown next to the nav arrows. */
  navLabel: string;
  onPrev: () => void;
  onNext: () => void;
  onToday: () => void;
  onGoToDate: (key: string) => void;
}

/**
 * The Agenda / Week toolbar — date nav only. A fixed flex sibling of the scroll region
 * (same treatment as the page header) so it stays put while the agenda scrolls
 * (specs/0041 WS6). The view switch lives in the Meetings header, the same place for all
 * four views. The old "Add to-do" and "All meetings" links are gone (owner request):
 * List is a view on this page, and Action items has its own sidebar entry. The inner
 * wrapper mirrors the scroll region's MEETINGS_CONTENT_WIDTH centering so nothing shifts.
 */
export function TodayToolbar({
  viewMode,
  viewDate,
  navLabel,
  onPrev,
  onNext,
  onToday,
  onGoToDate,
}: TodayToolbarProps) {
  const hasSlot = useHasNavSlot();
  const nav = (
    <DateNavigator
      label={navLabel}
      labelClassName="min-w-14"
      onPrev={onPrev}
      onNext={onNext}
      prevLabel={viewMode === 'week' ? 'Previous week' : 'Previous day'}
      nextLabel={viewMode === 'week' ? 'Next week' : 'Next day'}
      // Today is always a key, as in Week and Month (the DateNavigator default): the
      // agenda used to drop it and print "Today" as the label instead, so the same
      // control was drawn two ways across the views.
      onToday={onToday}
      trailing={
        // Native date picker: a transparent input overlays the calendar glyph so a click
        // opens the OS picker; the value stays a valid local YYYY-MM-DD. Same 28px ghost
        // key as the arrows beside it.
        <label className="relative flex h-7 w-7 cursor-pointer items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground focus-within:ring-2 focus-within:ring-ring">
          <DeckIcon icon={CalendarDays} size={14} />
          <Input
            type="date"
            aria-label="Jump to date"
            value={viewDate}
            onChange={(e) => e.target.value && onGoToDate(e.target.value)}
            className="absolute inset-0 h-full w-full cursor-pointer border-0 p-0 opacity-0 shadow-none"
          />
        </label>
      }
    />
  );
  // In the Meetings page the navigation sits at the right of the view-tab row (NavSlot);
  // standalone (tests) it keeps its own row.
  if (hasSlot) return <NavSlotPortal>{nav}</NavSlotPortal>;
  return (
    <div className={cn('flex-shrink-0 pt-1.5', PAGE_GUTTER)}>
      <div className={`mx-auto ${MEETINGS_CONTENT_WIDTH}`}>
        <div className="mb-2.5 flex flex-wrap items-center justify-between gap-x-4 gap-y-2 px-1">
          {/* Date navigation: prev / label + picker / next, plus a Today reset. */}
          {nav}
        </div>
      </div>
    </div>
  );
}
