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
  /** `bare`: inline-edit surface, no border or shadow, transparent, `bg-muted` on hover. */
  variant?: "default" | "bare"
}

const Input = React.forwardRef<HTMLInputElement, InputProps>(
  ({ className, type, size = "default", variant = "default", ...props }, ref) => {
    return (
      <input
        type={type}
        data-size={size}
        data-variant={variant}
        className={cn(
          "flex w-full rounded-md bg-transparent text-base transition-colors file:border-0 file:bg-transparent file:text-sm file:font-medium file:text-foreground placeholder:text-muted-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-50",
          variant === "bare"
            ? "h-auto border-0 text-sm px-1 py-0.5 shadow-none hover:bg-muted focus-visible:bg-muted"
            : "h-9 border border-input px-3 py-1 shadow-sm",
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
