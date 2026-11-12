import * as React from "react"
import { Slot } from "@radix-ui/react-slot"
import { cva, type VariantProps } from "class-variance-authority"

import { cn } from "@/lib/utils"

/** The dark, unlit state shared by the lamp variants (`brand`, `record`). */
const LAMP_DISABLED =
  "disabled:bg-muted disabled:text-muted-foreground disabled:shadow-none disabled:opacity-100"

const buttonVariants = cva(
  "inline-flex items-center justify-center gap-2 whitespace-nowrap rounded-md text-sm font-semibold transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:pointer-events-none disabled:opacity-50 [&_svg]:pointer-events-none [&_svg]:size-4 [&_svg]:shrink-0",
  {
    variants: {
      variant: {
        default:
          "bg-primary text-primary-foreground shadow hover:bg-primary/90",
        destructive:
          "bg-destructive text-destructive-foreground shadow-sm hover:bg-destructive/90",
        outline:
          "border border-input bg-background shadow-sm hover:bg-accent hover:text-accent-foreground",
        secondary:
          "bg-secondary text-secondary-foreground shadow-sm hover:bg-secondary/80",
        ghost: "hover:bg-accent hover:text-accent-foreground",
        link: "text-primary underline-offset-4 hover:underline",
        // A disabled lamp goes dark; it is never a dimmed lamp (docs/design-system.md
        // §Colour). `opacity-50` over amber still reads as a lit, pressable key in Deck.
        brand: `bg-brand text-brand-foreground shadow-sm hover:bg-brand/90 ${LAMP_DISABLED}`,
        // "+ Add …" affordances (docs/design-system.md): a dashed outline that lights to
        // the brand edge on hover. Was hand-rolled in ParticipantsPanel.
        dashed:
          "border border-dashed border-border bg-transparent text-muted-foreground hover:border-brand-edge hover:text-brand",
        // REC red (docs/design-system.md): starts a recording; nothing else. Errors and
        // destructive confirms are `destructive`, primary CTAs `brand`.
        record: `bg-record text-record-foreground shadow-sm hover:bg-record/90 ${LAMP_DISABLED}`,
      },
      size: {
        default: "h-9 px-4 py-2",
        sm: "h-8 px-3 text-xs",
        xs: "h-7 px-2.5 text-xs [&_svg]:size-3.5",
        lg: "h-10 px-8",
        icon: "h-9 w-9",
        "icon-sm": "h-7 w-7 [&_svg]:size-3.5",
      },
    },
    defaultVariants: {
      variant: "default",
      size: "default",
    },
  }
)

export interface ButtonProps
  extends React.ButtonHTMLAttributes<HTMLButtonElement>,
    VariantProps<typeof buttonVariants> {
  asChild?: boolean
}

const Button = React.forwardRef<HTMLButtonElement, ButtonProps>(
  ({ className, variant, size, asChild = false, ...props }, ref) => {
    const Comp = asChild ? Slot : "button"
    return (
      <Comp
        className={cn(buttonVariants({ variant, size, className }))}
        ref={ref}
        {...props}
      />
    )
  }
)
Button.displayName = "Button"

export { Button, buttonVariants }
