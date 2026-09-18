import React from 'react';
import { cn } from '@/lib/utils';
import { formatTalkTime, largestRemainderPercents } from '@/lib/speaker-talk-time';

export interface ChannelRow {
  channel: number;
  /** Index of the source speaker group, when the caller needs to map a row back to it. */
  index?: number;
  speakerKey: string;
  colorClass: string; // bg-chart-n
  seconds: number;
  share: number; // 0..1
}

const GRID = 'grid grid-cols-[36px_1fr_64px_minmax(120px,220px)_40px] items-center gap-x-3';

/**
 * specs/0057 §3.5 — a multitrack deck labels its inputs in a fixed-width column. Replaces the
 * speaker chip cloud: CH · color bar · name (the existing SpeakerChip, so rename/merge/assign
 * behave exactly as before) · time · share ladder · %.
 */
export function ChannelStrip({
  rows,
  renderName,
  className,
  selectedKey,
  onSelect,
}: {
  rows: ChannelRow[];
  renderName: (row: ChannelRow) => React.ReactNode;
  className?: string;
  /** The currently filtered speaker (specs/0061 W4 task 3), or null/undefined
   *  when nothing is selected. Only meaningful when `onSelect` is passed. */
  selectedKey?: string | null;
  /** Click (or Enter/Space while focused) a row to select it — the caller owns
   *  toggle-to-clear semantics; this just reports the key that was activated. */
  onSelect?: (key: string) => void;
}) {
  // Whole percentages allocated by largest remainder so the column adds to 100.
  const percents = largestRemainderPercents(rows.map((r) => r.share));
  return (
    <div
      role="table"
      aria-label="Channels"
      className={cn(
        'rounded-[3px] border border-border bg-card px-3.5 pb-2 pt-1.5',
        className,
      )}
    >
      <div role="row" className={cn(GRID, 'h-[22px]')}>
        <span role="columnheader" className="u-section-label text-[9px]">
          CH
        </span>
        <span role="columnheader" className="u-section-label text-[9px]">
          Speaker
        </span>
        <span role="columnheader" className="u-section-label text-right text-[9px]">
          Time
        </span>
        <span role="columnheader" className="u-section-label text-[9px]">
          Share of talk
        </span>
        {/* Spacer for the trailing action column — a columnheader with no name is noise. */}
        <span aria-hidden />
      </div>
      <div className="h-px bg-border" aria-hidden />
      {rows.map((r, i) => {
        const selected = onSelect ? r.speakerKey === selectedKey : false;
        // specs/0061 W4 task 3, ruling R4 — `aria-selected` only has option
        // semantics; a row wired for selection is exposed as a real toggle
        // button (`role="button"` + `aria-pressed`) instead. Rows with no
        // `onSelect` are left exactly as `role="row"` (unchanged).
        const interactiveProps = onSelect
          ? {
              role: 'button' as const,
              tabIndex: 0,
              'aria-pressed': selected,
              onClick: (e: React.MouseEvent<HTMLDivElement>) => {
                // A nested interactive control (rename button, merge menu
                // trigger, …) inside the "Speaker" cell owns its own click —
                // don't also fire the row's onSelect for it.
                const target = e.target as HTMLElement;
                const nestedInteractive = target.closest('button, a, input, [role="menuitem"]');
                if (nestedInteractive && nestedInteractive !== e.currentTarget) return;
                onSelect(r.speakerKey);
              },
              onKeyDown: (e: React.KeyboardEvent<HTMLDivElement>) => {
                // Only when the row itself (not a focused descendant control)
                // received the key — a nested button handles its own Enter/Space.
                if (e.target !== e.currentTarget) return;
                if (e.key === 'Enter' || e.key === ' ') {
                  e.preventDefault();
                  onSelect(r.speakerKey);
                }
              },
            }
          : { role: 'row' as const };
        return (
        <div
          key={r.speakerKey}
          data-testid="channel-row"
          className={cn(GRID, 'min-h-[26px]', onSelect && 'cursor-pointer', selected && 'bg-muted')}
          {...interactiveProps}
        >
          <span
            role="cell"
            className="flex items-center gap-2 text-[11px] font-semibold text-engrave"
          >
            <i className={cn('block h-3.5 w-[3px]', r.colorClass)} aria-hidden />
            {r.channel}
          </span>
          <span
            role="cell"
            className="min-w-0 overflow-hidden text-[13px] text-foreground"
          >
            {renderName(r)}
          </span>
          <span
            role="cell"
            className="text-right text-xs tabular-nums text-muted-foreground"
          >
            {formatTalkTime(r.seconds)}
          </span>
          <span
            role="cell"
            className="relative h-1.5 bg-well shadow-[inset_0_1px_1px_rgba(0,0,0,0.5)]"
          >
            <i
              className={cn('absolute inset-y-0 left-0', r.colorClass)}
              style={{ width: `${percents[i]}%` }}
              aria-hidden
            />
          </span>
          <span
            role="cell"
            className="text-right text-[11px] tabular-nums text-muted-foreground"
          >
            {percents[i]}%
          </span>
        </div>
        );
      })}
    </div>
  );
}
