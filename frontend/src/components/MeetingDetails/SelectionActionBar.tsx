'use client';

import { useEffect } from 'react';
import { UserPlus, X } from 'lucide-react';
import { Button } from '@/components/ui/button';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu';
import { speakerBgClass } from '@/lib/speaker-colors';

/** How far toasts rise while the bar is up, so a toast never lands on top of it: the bar is
 *  ~42px tall and sits 12px above the page bottom; toasts already clear the rail by 16px.
 *  Read by the Toaster's offset in `app/layout.tsx`. */
export const TOAST_LIFT_VAR = '--toast-lift';
const TOAST_LIFT = '48px';

/** Bottom padding the transcript adds while the bar is up, so the last lines scroll clear. */
export const SELECTION_BAR_CLEARANCE = 'pb-16';

interface SelectionActionBarProps {
  count: number;
  speakers: { speakerKey: string; displayName: string }[];
  onReassign: (speakerKey: string, displayName: string) => void;
  onNewSpeaker: () => void;
  onClear: () => void;
}

/**
 * specs/0039 WS2 — the span-reassignment action bar, shown while transcript lines are
 * selected. Owner feedback: it used to be positioned against the bottom of the WHOLE
 * transcript, and in meeting details the page column owns the scroll, so with a long
 * transcript it sat off-screen. It is now a zero-height `sticky bottom-0` anchor at the end
 * of the transcript, so the bar rides the bottom of the visible viewport wherever the user
 * has scrolled, and settles over the transcript's own foot at the end (where the caller adds
 * `SELECTION_BAR_CLEARANCE` so it covers nothing).
 *
 * Escape clears the selection unless the key belongs to something else (the reassign menu,
 * a dialog, a text field — those handle their own Escape).
 */
export function SelectionActionBar({
  count,
  speakers,
  onReassign,
  onNewSpeaker,
  onClear,
}: SelectionActionBarProps) {
  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key !== 'Escape' || e.defaultPrevented) return;
      const target = e.target instanceof Element ? e.target : null;
      if (
        target?.closest('[role="menu"], [role="dialog"], input, textarea, [contenteditable="true"]')
      ) {
        return;
      }
      onClear();
    };
    document.addEventListener('keydown', onKeyDown);
    return () => document.removeEventListener('keydown', onKeyDown);
  }, [onClear]);

  useEffect(() => {
    const root = document.documentElement;
    root.style.setProperty(TOAST_LIFT_VAR, TOAST_LIFT);
    return () => {
      root.style.removeProperty(TOAST_LIFT_VAR);
    };
  }, []);

  const lines = `${count} ${count === 1 ? 'line' : 'lines'}`;

  return (
    <div data-selection-bar-anchor className="pointer-events-none sticky bottom-0 z-20 h-0">
      <div
        role="toolbar"
        aria-label="Selected transcript lines"
        className="pointer-events-auto absolute bottom-3 left-1/2 flex -translate-x-1/2 items-center gap-1.5 whitespace-nowrap rounded-[3px] border border-border bg-panel px-2 py-1.5 shadow-lg"
      >
        <span className="pl-1.5 text-xs font-medium tabular-nums text-foreground">
          {lines} selected
        </span>
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button size="sm" className="h-7 px-2.5 text-xs">
              Reassign to…
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="center" side="top" className="w-56">
            <DropdownMenuLabel className="text-xs font-normal text-muted-foreground">
              Reassign {lines} to…
            </DropdownMenuLabel>
            <DropdownMenuSeparator />
            <div className="max-h-56 overflow-y-auto">
              {speakers.map((s) => (
                <DropdownMenuItem
                  key={s.speakerKey}
                  onSelect={() => onReassign(s.speakerKey, s.displayName)}
                  className="text-sm"
                >
                  <span
                    className={`mr-2 inline-block h-2.5 w-2.5 flex-shrink-0 rounded-full ${speakerBgClass(s.speakerKey)}`}
                    aria-hidden
                  />
                  <span className="flex-1 truncate">{s.displayName}</span>
                </DropdownMenuItem>
              ))}
            </div>
            <DropdownMenuSeparator />
            <DropdownMenuItem
              onSelect={(e) => {
                // Keep the selection; the caller opens the naming dialog.
                e.preventDefault();
                onNewSpeaker();
              }}
              className="text-sm"
            >
              <UserPlus size={13} className="mr-2 flex-shrink-0 text-muted-foreground" />
              New speaker…
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
        <button
          type="button"
          onClick={onClear}
          className="rounded-[3px] p-1 text-muted-foreground transition-colors hover:bg-muted hover:text-foreground"
          aria-label="Clear selection"
          title="Clear selection (Esc)"
        >
          <X size={14} />
        </button>
      </div>
    </div>
  );
}
