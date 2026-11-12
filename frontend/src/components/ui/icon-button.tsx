'use client';

/**
 * IconButton (specs/0063 W4) — an icon-only action whose label lives in a tooltip and in
 * the accessible name, never on screen.
 *
 * Use it where a labelled button would crowd a header that is not the point of the screen
 * (the Prep tab's "Link previous meeting…" / "Regenerate" sat above the brief and competed
 * with it). `rounded-md` (3px) matches the panel chrome — docs/design-system.md §Radius.
 *
 * `size` is `sm` (28px, the default) or `md` (32px) — the two control heights an icon
 * button may take. `variant` is `bordered` (the default: a card-faced key) or `ghost`
 * (no frame until hover, for toolbars and row overflow menus).
 *
 * It mounts its own `TooltipProvider`. One is already mounted app-wide in `app/layout.tsx`,
 * and nesting Radix providers is supported — but a component rendered in isolation (a test,
 * a Storybook-style harness) would otherwise throw, and an action button that explodes
 * outside the app shell is a trap for the next caller.
 */

import { forwardRef, type ButtonHTMLAttributes, type ReactNode } from 'react';
import { cva } from 'class-variance-authority';
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from '@/components/ui/tooltip';
import { cn } from '@/lib/utils';

export const iconButtonVariants = cva(
  [
    'inline-flex flex-none items-center justify-center rounded-md text-muted-foreground',
    'transition-colors hover:text-foreground',
    'focus:outline-none focus-visible:ring-2 focus-visible:ring-ring',
    'disabled:pointer-events-none disabled:opacity-60',
  ],
  {
    variants: {
      size: {
        sm: 'h-7 w-7',
        md: 'h-8 w-8',
      },
      variant: {
        bordered: 'border border-border bg-card hover:bg-muted',
        ghost: 'bg-transparent hover:bg-accent data-[state=open]:bg-accent data-[state=open]:text-foreground',
      },
    },
    defaultVariants: {
      size: 'sm',
      variant: 'bordered',
    },
  },
);

export type IconButtonSize = 'sm' | 'md';
export type IconButtonVariant = 'bordered' | 'ghost';

export interface IconButtonProps extends Omit<ButtonHTMLAttributes<HTMLButtonElement>, 'children'> {
  /** The full action text. Becomes the tooltip AND the button's accessible name. */
  label: string;
  icon: ReactNode;
  /** Where the tooltip sits relative to the button. */
  side?: 'top' | 'right' | 'bottom' | 'left';
  /** `sm` = 28px (default), `md` = 32px. */
  size?: IconButtonSize;
  /** `bordered` (default) or `ghost`. */
  variant?: IconButtonVariant;
}

export const IconButton = forwardRef<HTMLButtonElement, IconButtonProps>(function IconButton(
  { label, icon, side = 'top', size, variant, className, type = 'button', ...rest },
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
            data-variant={variant ?? 'bordered'}
            className={cn(iconButtonVariants({ size, variant }), className)}
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
