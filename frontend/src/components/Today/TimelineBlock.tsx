'use client';

import { Button } from '@/components/ui/button';
import type { DayAgendaItem } from '@/lib/day-agenda';
import { formatClockTime } from '@/lib/calendar';
import type { TimelineContext, TimelineVisualState } from '@/lib/today-timeline';
import { RecordingBadge } from '@/components/RecordingBadge';
import { ProcessingBadge } from '@/components/ProcessingBadge';
import { AgendaAttendees } from './AgendaAttendees';
import { AgendaRowMenu } from './AgendaRowMenu';

export const LANE_GAP_PX = 6; // horizontal gap between side-by-side (overlapping) blocks

/**
 * Per-state block chrome (specs/0057 Task 6). The timeline reads as a transport log:
 * every item is the same hairline panel, and its STATE is carried by the 3px left bar
 * (see `barClass`) rather than by a coloured glow. The one panel that differs is a
 * calendar event that was never recorded — a dashed outline over nothing, because
 * there is no tape.
 *
 * Exported (specs/0069 W4 fix round 1) so `DayList` paints the SAME per-state chrome —
 * notably the dashed/transparent "no tape" treatment for `past-unrecorded` — instead of
 * a single hardcoded style that silently drifts from the grid's.
 */
export function blockClasses(state: TimelineVisualState): string {
  switch (state) {
    case 'past-unrecorded':
      return 'border-dashed border-border/70 bg-transparent text-muted-foreground';
    case 'upcoming':
      return 'border-border bg-card hover:border-brand/40';
    case 'past-recorded':
    case 'processing':
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

/**
 * The 3px spine down the left edge — the block's state colour. `null` = no tape, no bar.
 * Exported so the List presentation (`DayList`, specs/0069 W4) paints the SAME spine
 * rather than re-deriving its own state→colour mapping.
 */
export function barClass(state: TimelineVisualState): string | null {
  switch (state) {
    case 'recording':
      return 'bg-record';
    case 'processing':
      return 'bg-brand';
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
 * Engraved caps rather than a pill. The live meeting's label IS the reels badge — the one
 * All Meetings shows — so every view marks it once, the same way (owner feedback
 * 2026-09-23: List and Week showed both the reels and an older lamp label).
 */
export function StateChip({ state }: { state: TimelineVisualState }): JSX.Element | null {
  const chip = (cls: string, label: string) => (
    <span className={`u-section-label flex-shrink-0 ${cls}`}>{label}</span>
  );
  switch (state) {
    case 'recording':
      return <RecordingBadge />;
    case 'processing':
      return <ProcessingBadge />;
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
  /** A manual, unrecorded entry with nothing else recording — offer the Record button. */
  canRecord: boolean;
  /** What the `…` menu offers (Hide / Edit / Delete) is derived from this, in
   *  `AgendaRowMenu` — so the grid, the list and the week can't disagree about it. */
  ctx: TimelineContext;
  onSelect: (item: DayAgendaItem) => void;
  onJoin: (item: DayAgendaItem) => void;
  onHide: (item: DayAgendaItem) => void;
  onEdit: (item: DayAgendaItem) => void;
  onDelete: (item: DayAgendaItem) => void;
  onRecord: (item: DayAgendaItem) => void;
}

export function TimelineBlock({
  item,
  state,
  top,
  height,
  lane,
  laneCount,
  canJoin,
  canRecord,
  ctx,
  onSelect,
  onJoin,
  onHide,
  onEdit,
  onDelete,
  onRecord,
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
        ) : canRecord ? (
          <Button
            variant="brand"
            size="xs"
            onClick={(e) => {
              e.stopPropagation();
              onRecord(item);
            }}
            className={`flex-shrink-0 ${compact ? 'h-5 px-2 text-[10.5px]' : ''}`}
          >
            Record
          </Button>
        ) : (
          <StateChip state={state} />
        )}
        <AgendaRowMenu item={item} ctx={ctx} actions={{ onHide, onEdit, onDelete, onRecord }} />
      </div>
      {!compact && (
        <div className="mt-0.5 flex min-w-0 items-center gap-2">
          <span className="u-meta flex-shrink-0">
            {validStart ? formatClockTime(start) : '--:--'}
          </span>
          {/* Faces instead of "· 3 attendees" — parity with All Meetings (owner feedback
              2026-09-21). A compact block has no meta line at all. */}
          <AgendaAttendees item={item} max={2} />
        </div>
      )}
    </div>
  );
}
