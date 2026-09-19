'use client';

/**
 * IconButton (specs/0063 W4) — an icon-only action whose label lives in a tooltip and in
 * the accessible name, never on screen.
 *
 * Use it where a labelled button would crowd a header that is not the point of the screen
 * (the Prep tab's "Link previous meeting…" / "Regenerate" sat above the brief and competed
 * with it). `rounded-[3px]` matches the panel chrome rather than the pill radius used by
 * primary actions.
 *
 * It mounts its own `TooltipProvider`. One is already mounted app-wide in `app/layout.tsx`,
 * and nesting Radix providers is supported — but a component rendered in isolation (a test,
 * a Storybook-style harness) would otherwise throw, and an action button that explodes
 * outside the app shell is a trap for the next caller.
 */

import { forwardRef, type ButtonHTMLAttributes, type ReactNode } from 'react';
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from '@/components/ui/tooltip';
import { cn } from '@/lib/utils';

export interface IconButtonProps extends Omit<ButtonHTMLAttributes<HTMLButtonElement>, 'children'> {
  /** The full action text. Becomes the tooltip AND the button's accessible name. */
  label: string;
  icon: ReactNode;
  /** Where the tooltip sits relative to the button. */
  side?: 'top' | 'right' | 'bottom' | 'left';
}

export const IconButton = forwardRef<HTMLButtonElement, IconButtonProps>(function IconButton(
  { label, icon, side = 'top', className, type = 'button', ...rest },
  ref,
) {
  return (
    <TooltipProvider delayDuration={300}>
      <Tooltip>
        <TooltipTrigger asChild>
          <button
            ref={ref}
            type={type}
            // No `title`: it would duplicate the tooltip as a second, slower native one.
            aria-label={label}
            className={cn(
              'inline-flex h-7 w-7 flex-none items-center justify-center rounded-[3px]',
              'border border-border bg-card text-muted-foreground',
              'transition-colors hover:bg-muted hover:text-foreground',
              'focus:outline-none focus-visible:ring-2 focus-visible:ring-ring',
              'disabled:pointer-events-none disabled:opacity-60',
              className,
            )}
            {...rest}
          >
            {icon}
          </button>
        </TooltipTrigger>
        <TooltipContent side={side}>{label}</TooltipContent>
      </Tooltip>
    </TooltipProvider>
  );
});
