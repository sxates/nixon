import * as React from "react"

import { Callout } from "@/components/ui/callout"
import { cn } from "@/lib/utils"

/**
 * Shim over `Callout` (design-system wave 0b) so the shadcn `Alert` callers keep working.
 * New code uses `Callout` with a `tone`. `variant="default"` is the plain bordered box on
 * the page background it always was; `destructive` is `Callout tone="error"`. A leading
 * `<svg>` child keeps the shadcn icon layout.
 */
const ICON_LAYOUT =
  "[&>svg+div]:translate-y-[-3px] [&>svg]:absolute [&>svg]:left-4 [&>svg]:top-4 [&>svg]:text-foreground [&>svg~*]:pl-7"

type AlertVariant = "default" | "destructive"

const Alert = React.forwardRef<
  HTMLDivElement,
  React.HTMLAttributes<HTMLDivElement> & { variant?: AlertVariant | null }
>(({ className, variant, ...props }, ref) => (
  <Callout
    ref={ref}
    role="alert"
    tone={variant === "destructive" ? "error" : "muted"}
    className={cn(
      ICON_LAYOUT,
      variant === "destructive"
        ? "text-destructive [&>svg]:text-destructive"
        : "border-border bg-background",
      className
    )}
    {...props}
  />
))
Alert.displayName = "Alert"

const AlertTitle = React.forwardRef<
  HTMLParagraphElement,
  React.HTMLAttributes<HTMLHeadingElement>
>(({ className, ...props }, ref) => (
  <h5
    ref={ref}
    className={cn("mb-1 font-medium leading-none tracking-tight", className)}
    {...props}
  />
))
AlertTitle.displayName = "AlertTitle"

const AlertDescription = React.forwardRef<
  HTMLParagraphElement,
  React.HTMLAttributes<HTMLParagraphElement>
>(({ className, ...props }, ref) => (
  <div
    ref={ref}
    className={cn("text-sm [&_p]:leading-relaxed", className)}
    {...props}
  />
))
AlertDescription.displayName = "AlertDescription"

export { Alert, AlertTitle, AlertDescription }
