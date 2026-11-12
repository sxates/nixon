"use client"

import * as React from "react"
import * as ProgressPrimitive from "@radix-ui/react-progress"

import { cn } from "@/lib/utils"

/**
 * Progress bar (docs/design-system.md): a 6px muted track with a brand (amber lamp) fill —
 * every download and move bar. `tone` switches the fill for a finished (`success`) or
 * failed (`destructive`) run. It used to fill ink-black `bg-primary`, which sat next to the
 * amber download bars looking like a different control.
 */
const TONE_FILL = {
  brand: "bg-brand",
  success: "bg-success",
  destructive: "bg-destructive",
} as const

export type ProgressTone = keyof typeof TONE_FILL

const Progress = React.forwardRef<
  React.ElementRef<typeof ProgressPrimitive.Root>,
  React.ComponentPropsWithoutRef<typeof ProgressPrimitive.Root> & {
    tone?: ProgressTone
  }
>(({ className, value, tone = "brand", ...props }, ref) => (
  <ProgressPrimitive.Root
    ref={ref}
    value={value}
    data-tone={tone}
    className={cn(
      "relative h-1.5 w-full overflow-hidden rounded-full bg-muted",
      className
    )}
    {...props}
  >
    <ProgressPrimitive.Indicator
      className={cn("h-full w-full flex-1 transition-all", TONE_FILL[tone])}
      style={{ transform: `translateX(-${100 - (value || 0)}%)` }}
    />
  </ProgressPrimitive.Root>
))
Progress.displayName = ProgressPrimitive.Root.displayName

export { Progress }
