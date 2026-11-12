"use client"

import * as React from "react"
import * as TabsPrimitive from "@radix-ui/react-tabs"

import { cn } from "@/lib/utils"

/**
 * Underline tabs (docs/design-system.md §Component catalog). The default style is the
 * deck's: a full-width hairline rule with the active tab owning a 2px brand underline —
 * the same formula as the meeting-details tab bar (MeetingTabsBar) and the Settings tabs,
 * which used to override the shadcn pill to get it. Radix supplies roving focus.
 */
const Tabs = TabsPrimitive.Root

const TabsList = React.forwardRef<
  React.ElementRef<typeof TabsPrimitive.List>,
  React.ComponentPropsWithoutRef<typeof TabsPrimitive.List>
>(({ className, ...props }, ref) => (
  <TabsPrimitive.List
    ref={ref}
    className={cn(
      "flex items-center gap-1 border-b border-border text-muted-foreground",
      className
    )}
    {...props}
  />
))
TabsList.displayName = TabsPrimitive.List.displayName

function TabBadge({ badge }: { badge: React.ReactNode }) {
  if (badge === undefined || badge === null || badge === false) return null
  if (typeof badge === "number" || typeof badge === "string") {
    return (
      <span
        data-tab-badge=""
        className="ml-1.5 rounded-sm bg-muted px-1 text-caption font-medium tabular-nums text-muted-foreground group-data-[state=active]:bg-brand-wash group-data-[state=active]:text-brand"
      >
        {badge}
      </span>
    )
  }
  return (
    <span data-tab-badge="" className="ml-1.5 inline-flex">
      {badge}
    </span>
  )
}

const TabsTrigger = React.forwardRef<
  React.ElementRef<typeof TabsPrimitive.Trigger>,
  React.ComponentPropsWithoutRef<typeof TabsPrimitive.Trigger> & {
    /**
     * Optional trailing badge — usually a count. A number or string renders as a small
     * count chip that lights brand on the active tab; any other node renders as given.
     */
    badge?: React.ReactNode
  }
>(({ className, badge, children, ...props }, ref) => (
  <TabsPrimitive.Trigger
    ref={ref}
    className={cn(
      "group -mb-px inline-flex items-center justify-center whitespace-nowrap border-b-2 border-transparent px-3 py-1.5 text-sm font-semibold text-muted-foreground transition-colors [transition-duration:140ms] hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:pointer-events-none disabled:opacity-50 data-[state=active]:border-brand data-[state=active]:text-foreground",
      className
    )}
    {...props}
  >
    {children}
    <TabBadge badge={badge} />
  </TabsPrimitive.Trigger>
))
TabsTrigger.displayName = TabsPrimitive.Trigger.displayName

const TabsContent = React.forwardRef<
  React.ElementRef<typeof TabsPrimitive.Content>,
  React.ComponentPropsWithoutRef<typeof TabsPrimitive.Content>
>(({ className, ...props }, ref) => (
  <TabsPrimitive.Content
    ref={ref}
    className={cn(
      "mt-2 ring-offset-background focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2",
      className
    )}
    {...props}
  />
))
TabsContent.displayName = TabsPrimitive.Content.displayName

export { Tabs, TabsList, TabsTrigger, TabsContent }
