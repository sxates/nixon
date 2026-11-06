'use client';

import { CalendarDays, ChevronLeft, ChevronRight } from 'lucide-react';
import type { AgendaMode } from '@/hooks/useDayAgenda';
import { MEETINGS_CONTENT_WIDTH } from '@/lib/meetings-view';
import { NavSlotPortal, useHasNavSlot } from '@/components/Meetings/NavSlot';

interface TodayToolbarProps {
  viewMode: AgendaMode;
  viewDate: string;
  /** Primary date label shown next to the nav arrows. */
  navLabel: string;
  viewIsToday: boolean;
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
  viewIsToday,
  onPrev,
  onNext,
  onToday,
  onGoToDate,
}: TodayToolbarProps) {
  const hasSlot = useHasNavSlot();
  const nav = (
          <div className="flex items-center gap-1">
            {/* Today sits LEFT of the arrows: the nav is right-aligned, so it appearing and
                disappearing never moves the arrows. */}
            {!(viewMode !== 'week' && viewIsToday) && (
              <button
                type="button"
                onClick={onToday}
                className="mr-1 rounded-md border border-border bg-card px-2.5 py-1 text-[12px] font-semibold text-foreground transition-colors hover:bg-accent focus:outline-none focus-visible:ring-2 focus-visible:ring-ring"
              >
                Today
              </button>
            )}
            <button
              type="button"
              onClick={onPrev}
              aria-label={viewMode === 'week' ? 'Previous week' : 'Previous day'}
              className="flex h-7 w-7 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground focus:outline-none focus-visible:ring-2 focus-visible:ring-ring"
            >
              <ChevronLeft className="h-4 w-4" aria-hidden="true" />
            </button>

            <div className="flex items-center gap-1.5">
              <h2 className="min-w-[3.5rem] text-center font-display text-[15px] font-semibold text-foreground">
                {navLabel}
              </h2>
              {/* Native date picker: a transparent input overlays the calendar icon so a
                  click opens the OS picker; the value stays a valid local YYYY-MM-DD. */}
              <label className="relative flex h-7 w-7 cursor-pointer items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground focus-within:ring-2 focus-within:ring-ring">
                <CalendarDays className="h-[15px] w-[15px]" aria-hidden="true" />
                <input
                  type="date"
                  aria-label="Jump to date"
                  value={viewDate}
                  onChange={(e) => e.target.value && onGoToDate(e.target.value)}
                  className="absolute inset-0 cursor-pointer opacity-0"
                />
              </label>
            </div>

            <button
              type="button"
              onClick={onNext}
              aria-label={viewMode === 'week' ? 'Next week' : 'Next day'}
              className="flex h-7 w-7 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground focus:outline-none focus-visible:ring-2 focus-visible:ring-ring"
            >
              <ChevronRight className="h-4 w-4" aria-hidden="true" />
            </button>
          </div>
  );
  // In the Meetings page the navigation sits at the right of the view-tab row (NavSlot);
  // standalone (tests) it keeps its own row.
  if (hasSlot) return <NavSlotPortal>{nav}</NavSlotPortal>;
  return (
    <div className="flex-shrink-0 px-7 pt-1.5">
      <div className={`mx-auto ${MEETINGS_CONTENT_WIDTH}`}>
        <div className="mb-2.5 flex flex-wrap items-center justify-between gap-x-4 gap-y-2 px-1">
          {/* Date navigation: prev / label + picker / next, plus a Today reset. */}
          {nav}
        </div>
      </div>
    </div>
  );
}
