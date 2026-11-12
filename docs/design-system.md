# Nixon design system

The contract every UI change is built and reviewed against. The look is a 1970s tape-deck
faceplate (specs/0057): **Faceplate** (light, default) and **Deck** (`.dark`). The rules are
enforced by `scripts/check-ui-consistency.sh` (ADR-0019) and reviewed by the `ux-designer`
agent, which uses this file as its rubric.

**The one rule:** build screens from `frontend/src/components/ui/`. If a primitive doesn't
fit, extend the primitive (a variant, a size, a slot). Don't hand-roll a lookalike next to it.

## 1. Tokens

### Colour

Semantic tokens only (`check-off-token-colors.sh`). Raw palette classes and hex literals
don't flip with `.dark`.

| Role | Token |
|---|---|
| Page / panel surface | `bg-background`, `bg-panel` |
| Card, popover | `bg-card`, `bg-popover` |
| Body text / secondary text | `text-foreground` / `text-muted-foreground` |
| Engraved silkscreen label | `text-engrave` |
| Hairline | `border-border` (`border-rule` for the soft in-card divider) |
| Transcript paper | `bg-paper`, `text-paper-ink` |
| **Amber lamp**: armed, active, selected, queue, today, primary CTA | `brand` |
| **REC red**: recording state *only* | `record`, `text-record-ink` |
| **Ready green**: granted, done | `success` |
| Error, destructive confirm | `destructive` |

Lamp colours are the only saturated ones, and each means one thing. Red never marks a
non-recording CTA. Use `destructive` for errors, never `record`.

**Tints.** Use the named washes, not ad-hoc opacities:

| Token | Use |
|---|---|
| `bg-brand-wash` (brand 10%) | selected row, active badge fill |
| `border-brand-edge` (brand 30%) | today/active outline |
| `bg-destructive-wash` / `border-destructive-edge` (5% / 30%) | error callout |
| `bg-success-wash` (10%) | granted badge |

Hover states come from the primitive's variant. `bg-brand/90` in a screen means someone
hand-rolled a button.

### Type scale

Named sizes in `tailwind.config.js` `fontSize`. **No `text-[Npx]` in screens.**

| Class | Size | Role |
|---|---|---|
| `text-micro` | 9px | key caps, chips, table-header legends (`.u-section-label-sm`) |
| `text-label` | 10px | engraved section label (`.u-section-label`) |
| `text-caption` | 11px | row meta: times, emails, counts, badge text (`.u-row-meta`) |
| `text-xs` | 12px | sublines, secondary buttons |
| `text-meta` | 13px | metadata line (`.u-meta`) |
| `text-body` | 13.5px | list-row titles (`.u-row-title`), typed transcript (`.u-typed`) |
| `text-sm` | 14px | controls, body copy |
| `text-lead` | 15px | Ask input, toolbar heading |
| `text-heading` | 18px | document section heading (`.u-doc-heading`) |
| `text-title` | 22px | page / meeting title (`.u-page-title`) |
| `text-display` | 30px | onboarding hero, About wordmark |

Text-role utilities in `globals.css` combine size, weight, case, tracking and colour. Use
them rather than recombining the parts:

- `.u-section-label`: 10px / 600 / caps / 0.14em / engrave, with the engraved shadow.
- `.u-section-label-sm`: the same at 9px. Never shrink `.u-section-label` with `text-[…]`.
- `.u-row-title`: 13.5px / 600 / foreground.
- `.u-row-meta`: 11px / muted.
- `.u-meta`: 13px / muted.
- `.u-speaker`: transcript speaker name (typed, 12px, bold, caps).
- `.u-page-title`: 22px display.
- `.u-doc-heading`: 18px display.
- `.u-typed`: transcript body.

Weights: headings and row titles are `font-semibold`. Row and subgroup headings in Settings
are `font-medium`. `font-bold` belongs only to `.u-speaker`.

Fonts: Archivo (`font-sans`, `font-display`) for UI; Archivo Narrow (`font-narrow`) for meter
scales; Plex Sans (`font-reading`) for documents; Plex Mono (`font-mono`) for timecodes;
Courier Prime (`font-type`) for the transcript.

### Radius

Corners are hardware-tight (`--radius` = 4px).

| Class | Size | Use |
|---|---|---|
| `rounded-sm` | 2px | chips, badges, key caps, switch thumb |
| `rounded-md` | 3px | buttons, inputs, rows, menu items |
| `rounded-lg` | 4px | cards, panels, callouts, dialogs, popovers |
| `rounded-full` | | avatars, status dots and lamps, progress tracks **only** |

Spell them this way. `rounded` (4px), `rounded-[3px]` and `rounded-[2px]` are the same
values spelled differently, and `rounded-xl`/`2xl` are off-brand.

### Elevation

Settings and lists are flat. Cards use `shadow-card`. Popovers and dialogs keep the
primitive's shadow. No arbitrary `shadow-[…]`.

### Spacing and layout

- Page column: `PAGE_CONTENT_WIDTH` (1080px) and `PAGE_GUTTER` (`px-4 min-[900px]:px-7`)
  from `lib/page-width.ts`. The `PageHeader` sits inside the same column as the content, so
  the title, tabs and first card share one left edge.
- Document column: `DOC_CONTENT_WIDTH`. Never a bare `max-w-[Npx]` on a page column.
- Rhythm: section gap `mt-8`; panel padding `px-4 py-3` (`p-6` for documents); row padding
  `px-3 py-2`; list stack `gap-0.5`; card stack `gap-3`.
- Control heights: 28 (`xs`), 32 (`sm`), 36 (`default`). Nothing in between.

### Icons

Sparse, mechanical. Icons are imported from `@/components/ui/icons` (never straight from
`lucide-react`) and rendered through `DeckIcon` at 12, 14 (default) or 16px. The barrel *is*
the allowlist: adding a glyph to it is a reviewed design decision.

- **Functional glyphs only:** back, chevrons, close, check, overflow `…` (horizontal), play,
  pause, spinner, star toggle, refresh, and icon-only trash.
- **Allowed:** leading icons on dropdown menu items (`DropdownMenuItem icon={…}`), and the
  `Sparkles` glyph on AI actions (Ask, Summarize, Generate). It marks "this uses the model".
- **Not allowed:** decorative icons on card headers, tabs, text buttons, onboarding steps,
  list rows, or empty states (no icon discs). Status is a `StatusDot`, not a glyph.

### Motion

A short opacity fade (`.animate-fade-in`, 140ms). No travel: no `y:`/`x:` entrance offsets.
Continuous animations respect calm motion (specs/0077).

### Focus

Every interactive element shows `focus-visible:ring-2 ring-ring`. Never `focus:outline-none`
without a visible ring.

## 2. Component catalog (`frontend/src/components/ui/`)

| Need | Use | Notes |
|---|---|---|
| Any button with text | `Button` | variants `brand` (primary CTA), `outline` (row action), `ghost`, `link`, `dashed` ("+ Add …"), `destructive` (**confirm inside a dialog only**); sizes `xs`/`sm`/`default`/`lg`, plus `icon-sm` (28px square) |
| Icon-only button | `IconButton` | `size` `sm` (28) / `md` (32), `variant` `ghost` / `bordered`, tooltip from `label` |
| Overflow `…` | `MenuButton` | an `IconButton` preset: 14px horizontal glyph, opens a `DropdownMenu` |
| Menu items | `DropdownMenuItem` | `icon` prop; `destructive` variant |
| Page title row | `PageHeader` | `back`, editable `title`, `subtitle` node (not inside the h1), `actions` |
| Inline editable text | `EditableText` | the click-to-edit title or field, with the hover pencil |
| Underline tabs | `Tabs` / `TabStrip` | underline is the default; optional `badge` slot; roving focus built in |
| View switch | `SegmentedControl` | `size` default / `compact`; the only segmented style |
| Card | `Panel` | `title`, `count`, `action` slot, `variant` `solid` / `dashed`; header is a `.u-section-label` |
| Inline notice | `Callout` | `tone` `info` / `warn` / `error` / `muted`, `title`, `action`; replaces `Alert` and the ad-hoc error boxes (`SettingsNote` is a thin alias) |
| Empty / loading / error | `EmptyState`, `PageLoading`, `Callout tone="error"` | no icon discs |
| Run in progress | `RunStatus` | spinner, label and an optional Cancel, inside a `Panel` |
| Spinner | `Spinner` | `sm` (14) / `md` (20), inherits colour; `label` makes it a `role="status"`; never a raw `Loader2` or a CSS spinner |
| Progress bar | `Progress` | `h-1.5 rounded-full bg-muted` with a `bg-brand` fill; every download and move bar; `tone` `brand` / `success` / `destructive` |
| Status pill | `Badge` | `tone` `brand` / `success` / `muted` / `destructive`; 11px, `rounded-sm` |
| Lamp / status / speaker dot | `StatusDot` | `size` 6/8/10; `tone` or a speaker colour class |
| Person | `Avatar`, `AvatarStack` | photo or initials; the only `initials()` |
| Person picker | `PersonPicker` / `PersonOption` | search, then name with muted email and a suffix |
| List row | `ListRow` | slots `lead`, `title`, `meta`, `trailing`, `menu`; `LinkRow` adds the chevron |
| Settings | `SettingsSection` (`action` slot) / `SettingsGroup` / `SettingsRow` | rows put `Button variant="outline" size="sm"` on the right |
| Permission | `PermissionRow` + `PermissionStatus` | one shape for settings, onboarding and the modal |
| Selectable card | `OptionCard` | model pickers, theme choice |
| Confirm | `ConfirmDialog` | `title`, `body`, `confirmLabel`, `tone`, `busy` |
| Date navigation | `DateNavigator` | Today / prev / label / next |
| Form field | `Input`, `Textarea`, `Select`, `Switch`, `Label` (`optional` prop) | no native `<input>`/`<select>` in screens |

Domain components (`ProcessingBadge`, `RecordingBadge`, `StateChip`, `LampDot`, `TapeCounter`,
`VuMeter`, `TransportKey`) are composed from these and stay where they are.

## 3. Exceptions

Hardware-faithful parts of the deck (`Transport/` tape counter, VU meter, transport keys,
`ChannelStrip` scale) may use raw sizes and elements. Each exception sits on an allowlist in
`scripts/ui-consistency-allow.txt` with a reason. A one-off exception on a single line uses
`// ui-gate: allow <reason>`.

## 4. Changing the system

Adding a token, variant or primitive is fine. Do it in `components/ui/` or `globals.css`,
add it to this file in the same change, and run `pnpm shots` then `pnpm shots:diff`.
Changing what an existing token *means* (a lamp colour's role, the radius scale) needs an
ADR.
