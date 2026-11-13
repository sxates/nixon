import * as React from "react"

import { cn } from "@/lib/utils"

/**
 * Multi-line text field. `variant="bare"` is an inline-editable surface (no border,
 * transparent, no shadow; a `bg-muted` wash on hover) for titles that edit in place;
 * `size="title"` sets it in the page-title type (`text-title`).
 */
export interface TextareaProps extends React.ComponentProps<"textarea"> {
  variant?: "default" | "bare"
  size?: "default" | "title"
}

const Textarea = React.forwardRef<HTMLTextAreaElement, TextareaProps>(
  ({ className, variant = "default", size = "default", ...props }, ref) => {
    return (
      <textarea
        data-variant={variant}
        className={cn(
          "flex w-full rounded-md placeholder:text-muted-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-50",
          variant === "bare"
            ? "min-h-0 resize-none overflow-hidden border-0 bg-transparent px-1 py-0.5 shadow-none transition-colors hover:bg-muted"
            : "min-h-[60px] border border-input bg-transparent px-3 py-2 shadow-sm",
          size === "title"
            ? "font-display text-title font-semibold"
            : "text-base md:text-sm",
          className
        )}
        ref={ref}
        {...props}
      />
    )
  }
)
Textarea.displayName = "Textarea"

export { Textarea }
