# Vinyl UI — conventions for building with these components

Vinyl is a local-first macOS meeting assistant. These are its shadcn/ui primitives
(Radix + Tailwind + CSS-variable tokens). Build real, shippable UI with them.

## Setup & wrapping

- **No theme provider is required for styling.** Components are styled by the shipped
  `styles.css`, which defines the design tokens as CSS variables on `:root`. Make sure
  `styles.css` is loaded; everything else is utility classes + tokens.
- **Dark mode**: add `class="dark"` to an ancestor (e.g. `<body>` or a wrapper). The
  `.dark` block redefines every token; no prop changes needed.
- **`Tooltip` requires a `TooltipProvider` ancestor** — wrap the app (or the tooltip's
  region) once: `<TooltipProvider> … </TooltipProvider>`. Without it the tooltip throws.
- **`Dialog` / `Select` render in a portal** with an overlay; control open state with
  `defaultOpen` / `open` on `<Dialog>` and `defaultValue` / `value` on `<Select>`.

## Styling idiom — Tailwind utilities + semantic token classes

Style with Tailwind utility classes. For color, **use the semantic token classes** (never
raw hex) so light/dark and rebrands flow automatically:

| Surface | Class |
|---|---|
| App background / text | `bg-background` / `text-foreground` |
| Primary action | `bg-primary` / `text-primary-foreground` |
| Subtle surface | `bg-secondary` / `text-secondary-foreground`, `bg-muted` / `text-muted-foreground` |
| Hover / accent surface | `bg-accent` / `text-accent-foreground` |
| Card / popover | `bg-card`, `bg-popover` (+ `-foreground`) |
| Danger | `bg-destructive` / `text-destructive-foreground` |
| Borders / focus ring | `border-border`, `ring-ring` |
| Radius | `rounded-md` / `rounded-lg` (driven by `--radius`) |

Layout glue (`flex`, `grid`, `grid-cols-*`, `gap-*`, `p*-*`, `w-*`, `text-sm`,
`font-medium`, `shadow`, etc.) is available in the shipped CSS.

**Prefer component props over re-styling.** `Button` carries the design language via
`variant` — `default | secondary | outline | ghost | destructive | link` plus Vinyl's
brand set `blue` (Join & Record), `green` (Summarize), `red` (Stop), `gray` — and
`size` (`sm | default | lg | icon`). Reach for `<Button variant="blue">` before hand-styling.

## Where the truth lives

- Tokens + utilities: the bound `_ds/<folder>/styles.css` (read it before styling).
- Per-component API + usage: each component's `.d.ts` (props) and `.prompt.md` (how to compose).

## Idiomatic snippet

```tsx
<TooltipProvider>
  <div className="flex flex-col gap-3 rounded-lg border border-border bg-card p-4">
    <label className="text-sm font-medium text-foreground">Meeting title</label>
    <Input placeholder="Untitled meeting" />
    <div className="flex justify-end gap-2">
      <Button variant="outline">Cancel</Button>
      <Button variant="blue">Join &amp; Record</Button>
    </div>
  </div>
</TooltipProvider>
```
