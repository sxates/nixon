import * as React from "react"

import { cn } from "@/lib/utils"

/**
 * Text input (docs/design-system.md §Component catalog). `size` is `default` (36px, 14px
 * text) or `lg` (36px, `text-lead` 15px — the Ask question field). The native numeric
 * `size` attribute is not exposed; width comes from layout classes.
 */
const INPUT_SIZES = {
  default: "md:text-sm",
  lg: "text-lead md:text-lead",
} as const

export type InputSize = keyof typeof INPUT_SIZES

export interface InputProps extends Omit<React.ComponentProps<"input">, "size"> {
  size?: InputSize
}

const Input = React.forwardRef<HTMLInputElement, InputProps>(
  ({ className, type, size = "default", ...props }, ref) => {
    return (
      <input
        type={type}
        data-size={size}
        className={cn(
          "flex h-9 w-full rounded-md border border-input bg-transparent px-3 py-1 text-base shadow-sm transition-colors file:border-0 file:bg-transparent file:text-sm file:font-medium file:text-foreground placeholder:text-muted-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-50",
          INPUT_SIZES[size],
          className
        )}
        ref={ref}
        {...props}
      />
    )
  }
)
Input.displayName = "Input"

export { Input }
