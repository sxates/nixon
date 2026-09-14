'use client';

import { useState } from 'react';
import { useRouter } from 'next/navigation';
import { ArrowRight, CalendarDays, ChevronLeft, ChevronRight, ListPlus } from 'lucide-react';
import { toast } from 'sonner';
import { AddActionItemRow } from '@/components/ActionItems/AddActionItemRow';
import { useActionItemMutations } from '@/hooks/useActionItemMutations';
import type { ActionItem } from '@/types';
import type { AgendaViewMode } from '@/hooks/useDayAgenda';
import { SegmentedControl } from '@/components/ui/segmented-control';

const AGENDA_VIEWS = [
  { value: 'day', label: 'Day' },
  { value: 'week', label: 'Week' },
];

interface TodayToolbarProps {
  viewMode: AgendaViewMode;
  viewDate: string;
  /** Primary date label shown next to the nav arrows. */
  navLabel: string;
  viewIsToday: boolean;
  onPrev: () => void;
  onNext: () => void;
  onToday: () => void;
  onGoToDate: (key: string) => void;
  onSwitchMode: (mode: AgendaViewMode) => void;
}

/**
 * Home toolbar — date nav + Day/Week toggle + quick actions. A fixed flex sibling of
 * the scroll region (same treatment as the greeting header) so it stays put while
 * the agenda scrolls (specs/0041 WS6). Serves both day and week modes. The inner
 * wrapper mirrors the scroll region's max-w-[840px] centering so nothing shifts.
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
  onSwitchMode,
}: TodayToolbarProps) {
  const router = useRouter();

  // WS1.d — lightweight "add a to-do" affordance. Creates a STANDALONE manual action
  // item (no meetingId) via the shared create mutation, then toasts a link to the hub.
  // The list itself lives on /tasks, so we don't render created items here (scratch state
  // + appendCreated:false); the mutation is reused, not duplicated.
  const [showTodo, setShowTodo] = useState(false);
  const [, setTodoScratch] = useState<ActionItem[]>([]);
  const { add: addTodo } = useActionItemMutations<ActionItem>({
    setItems: setTodoScratch,
    meetingId: null,
    appendCreated: () => false,
    onAdded: () =>
      toast.success('To-do added', {
        description: 'Find it in Action items.',
        action: { label: 'Open', onClick: () => router.push('/tasks') },
      }),
  });

  return (
    <div className="flex-shrink-0 px-7 pt-1.5">
      <div className="mx-auto max-w-[840px]">
        <div className="mb-2.5 flex flex-wrap items-center justify-between gap-x-4 gap-y-2 px-1">
          {/* Date navigation: prev / label + picker / next, plus a Today reset. */}
          <div className="flex items-center gap-1">
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

            {!(viewMode === 'day' && viewIsToday) && (
              <button
                type="button"
                onClick={onToday}
                className="ml-1 rounded-md border border-border bg-card px-2.5 py-1 text-[12px] font-semibold text-foreground transition-colors hover:bg-accent focus:outline-none focus-visible:ring-2 focus-visible:ring-ring"
              >
                Today
              </button>
            )}
          </div>

          <div className="flex items-center gap-3">
            {/* Day / Week toggle — the shared underlined segmented control. */}
            <SegmentedControl
              aria-label="Agenda view"
              options={AGENDA_VIEWS}
              value={viewMode}
              onChange={(next) => onSwitchMode(next as 'day' | 'week')}
            />

            <button
              type="button"
              onClick={() => setShowTodo((v) => !v)}
              aria-expanded={showTodo}
              className="inline-flex items-center gap-1 rounded text-[12.5px] font-semibold text-muted-foreground hover:text-foreground focus:outline-none focus-visible:ring-2 focus-visible:ring-ring"
            >
              <ListPlus className="h-3.5 w-3.5" aria-hidden="true" />
              Add to-do
            </button>
            <button
              type="button"
              onClick={() => router.push('/meetings')}
              className="inline-flex items-center gap-1 rounded text-[12.5px] font-semibold text-brand hover:underline focus:outline-none focus-visible:ring-2 focus-visible:ring-ring"
            >
              All meetings
              <ArrowRight className="h-3.5 w-3.5" aria-hidden="true" />
            </button>
          </div>
        </div>

        {/* The Add-to-do input belongs to the toolbar's action, so it lives in the
            fixed region too — toggling it while scrolled must not open it off-screen. */}
        {showTodo && (
          <div className="mb-3 px-1">
            <AddActionItemRow
              placeholder="Add a to-do — it lands in Action items…"
              onAdd={addTodo}
            />
          </div>
        )}
      </div>
    </div>
  );
}
