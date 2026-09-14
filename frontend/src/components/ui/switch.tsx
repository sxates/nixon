"use client"

import * as React from "react"
import * as SwitchPrimitives from "@radix-ui/react-switch"

import { cn } from "@/lib/utils"

/**
 * Nixon's 2-position toggle (specs/0057 Task 2): a recessed rectangular well with
 * a hard-edged thumb that snaps between two detents, not an iOS pill. Still the
 * Radix switch underneath — every prop and the `role="switch"` contract are
 * unchanged, because 16 call sites and the settings tests depend on them.
 */
const Switch = React.forwardRef<
  React.ElementRef<typeof SwitchPrimitives.Root>,
  React.ComponentPropsWithoutRef<typeof SwitchPrimitives.Root>
>(({ className, ...props }, ref) => (
  <SwitchPrimitives.Root
    className={cn(
      "peer inline-flex h-[18px] w-8 shrink-0 cursor-pointer items-center rounded-[3px] bg-well p-[2px] shadow-[inset_0_1px_2px_rgba(0,0,0,0.35)] transition-colors [transition-duration:60ms] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background disabled:cursor-not-allowed disabled:opacity-50 data-[state=checked]:bg-brand/25",
      className
    )}
    {...props}
    ref={ref}
  >
    <SwitchPrimitives.Thumb
      className={cn(
        "pointer-events-none block h-3.5 w-3.5 rounded-[2px] bg-foreground shadow-sm ring-0 transition-transform [transition-duration:60ms] data-[state=checked]:translate-x-[14px] data-[state=unchecked]:translate-x-0"
      )}
    />
  </SwitchPrimitives.Root>
))
Switch.displayName = SwitchPrimitives.Root.displayName

export { Switch }
