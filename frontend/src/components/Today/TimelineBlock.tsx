'use client';

import { MoreHorizontal, EyeOff } from 'lucide-react';
import { Button } from '@/components/ui/button';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu';
import type { DayAgendaItem } from '@/lib/day-agenda';
import { formatClockTime } from '@/lib/calendar';
import type { TimelineVisualState } from '@/lib/today-timeline';

export const LANE_GAP_PX = 6; // horizontal gap between side-by-side (overlapping) blocks

/**
 * Per-state block chrome (specs/0057 Task 6). The timeline reads as a transport log:
 * every item is the same hairline panel, and its STATE is carried by the 3px left bar
 * (see `barClass`) rather than by a coloured glow. The one panel that differs is a
 * calendar event that was never recorded — a dashed outline over nothing, because
 * there is no tape.
 */
function blockClasses(state: TimelineVisualState): string {
  switch (state) {
    case 'past-unrecorded':
      return 'border-dashed border-border/70 bg-transparent text-muted-foreground';
    case 'upcoming':
      return 'border-border bg-card hover:border-brand/40';
    case 'past-recorded':
      return 'border-border bg-card hover:border-brand/40';
    case 'recording':
    case 'now-joinable':
    case 'now':
      return 'border-border bg-card';
    default: {
      const _x: never = state;
      return _x;
    }
  }
}

/** The 3px spine down the left edge — the block's state colour. `null` = no tape, no bar. */
function barClass(state: TimelineVisualState): string | null {
  switch (state) {
    case 'recording':
      return 'bg-record';
    case 'now-joinable':
      return 'bg-brand';
    case 'now':
      return 'bg-brand/50';
    case 'upcoming':
      return 'bg-muted-foreground/40';
    case 'past-recorded':
      return 'bg-success';
    case 'past-unrecorded':
      return null;
    default: {
      const _x: never = state;
      return _x;
    }
  }
}

/**
 * Short right-aligned status label per state (none for a missed/unrecorded event).
 * Engraved caps rather than a pill: the rail's turning reels carry liveness now, so the
 * recording label needs no blink — just a steady square record lamp.
 */
export function StateChip({ state }: { state: TimelineVisualState }): JSX.Element | null {
  const chip = (cls: string, label: string) => (
    <span className={`u-section-label flex-shrink-0 ${cls}`}>{label}</span>
  );
  switch (state) {
    case 'recording':
      return (
        <span className="u-section-label inline-flex flex-shrink-0 items-center gap-1.5 text-record">
          <span className="h-1.5 w-1.5 flex-shrink-0 bg-record" aria-hidden="true" />
          Recording…
        </span>
      );
    case 'now-joinable':
      return chip('text-brand', 'Join & record');
    case 'now':
      return chip('text-brand', 'Now');
    case 'upcoming':
      return chip('text-muted-foreground', 'Prep');
    case 'past-recorded':
      return chip('text-success', 'Recorded');
    case 'past-unrecorded':
      return null;
  }
}

interface TimelineBlockProps {
  item: DayAgendaItem;
  state: TimelineVisualState;
  top: number;
  height: number;
  lane: number;
  laneCount: number;
  canJoin: boolean;
  /** Unrecorded calendar item — offer "Hide from timeline" (specs/0026 + 0041 WS5). */
  canHide: boolean;
  onSelect: (item: DayAgendaItem) => void;
  onJoin: (item: DayAgendaItem) => void;
  onHide: (item: DayAgendaItem) => void;
}

export function TimelineBlock({
  item,
  state,
  top,
  height,
  lane,
  laneCount,
  canJoin,
  canHide,
  onSelect,
  onJoin,
  onHide,
}: TimelineBlockProps) {
  const start = new Date(item.startTime);
  const validStart = !Number.isNaN(start.getTime());
  const title = item.title?.trim() || 'Untitled meeting';
  // Percentage-based lane columns with a fixed gap carved out on the right.
  const widthPct = 100 / laneCount;
  const compact = height < 46;
  const bar = barClass(state);

  // The whole block is clickable (view / open Prep — never auto-joins); a live meeting
  // carries a separate Join & Record button that stops propagation.
  const handleKeyDown = (e: React.KeyboardEvent<HTMLDivElement>) => {
    if (e.key === 'Enter' || e.key === ' ') {
      e.preventDefault();
      onSelect(item);
    }
  };

  return (
    <div
      role="button"
      tabIndex={0}
      onClick={() => onSelect(item)}
      onKeyDown={handleKeyDown}
      title={title}
      style={{
        top,
        height,
        left: `${lane * widthPct}%`,
        width: `calc(${widthPct}% - ${LANE_GAP_PX}px)`,
      }}
      className={`group absolute cursor-pointer overflow-hidden rounded-[3px] border px-3 py-1.5 text-left transition-colors focus:outline-none focus-visible:ring-2 focus-visible:ring-ring ${blockClasses(
        state,
      )}`}
    >
      {bar && <i aria-hidden="true" data-state-bar className={`absolute inset-y-0 left-0 w-[3px] ${bar}`} />}
      <div className="flex items-center gap-2">
        <span
          className={`min-w-0 flex-1 truncate font-semibold ${compact ? 'text-[12.5px]' : 'text-[13.5px]'} ${
            state === 'past-unrecorded' ? 'text-muted-foreground' : 'text-foreground'
          }`}
        >
          {title}
        </span>
        {canJoin ? (
          <Button
            variant="brand"
            size="xs"
            onClick={(e) => {
              e.stopPropagation();
              onJoin(item);
            }}
            className={`flex-shrink-0 ${compact ? 'h-5 px-2 text-[10.5px]' : ''}`}
          >
            Join &amp; record
          </Button>
        ) : (
          <StateChip state={state} />
        )}
        {canHide && (
          /* Hover ⋯ menu (house idiom: All-meetings row options). The wrapper stops
             click/keyboard propagation so opening the menu never routes the block. */
          <span
            onClick={(e) => e.stopPropagation()}
            onKeyDown={(e) => e.stopPropagation()}
            className="flex-shrink-0"
          >
            <DropdownMenu>
              <DropdownMenuTrigger asChild>
                <button
                  type="button"
                  aria-label="Event options"
                  title="Event options"
                  className="inline-flex h-5 w-5 items-center justify-center rounded-md text-muted-foreground opacity-0 transition-opacity hover:bg-accent hover:text-foreground focus:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:opacity-100 group-hover:opacity-100 data-[state=open]:opacity-100"
                >
                  <MoreHorizontal className="h-3.5 w-3.5" />
                </button>
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end">
                <DropdownMenuItem onSelect={() => onHide(item)}>
                  <EyeOff className="mr-2 h-4 w-4" />
                  Hide from timeline
                </DropdownMenuItem>
              </DropdownMenuContent>
            </DropdownMenu>
          </span>
        )}
      </div>
      {!compact && (
        <div className="u-meta mt-0.5 truncate">
          {validStart ? formatClockTime(start) : '--:--'}
          {item.attendeeCount > 0 && ` · ${item.attendeeCount} attendee${item.attendeeCount === 1 ? '' : 's'}`}
        </div>
      )}
    </div>
  );
}
