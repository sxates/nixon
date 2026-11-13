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
| Dialog overlay | `bg-scrim` (its own alpha per theme) |
| Transcript paper | `bg-paper`, `text-paper-ink` |
| **Amber lamp**: armed, active, selected, queue, today, primary CTA | `brand` |
| **REC red**: recording state, and the control that starts one (`Button variant="record"`) | `record`, `text-record-ink` |
| **Ready green**: granted, done | `success` |
| Error, destructive confirm | `destructive` |

Lamp colours are the only saturated ones, and each means one thing. Red never marks a
non-recording CTA. Use `destructive` for errors, never `record`.

**A disabled lamp goes dark; it is never a dimmed lamp.** A disabled `brand` or `record`
button is drawn unlit (`bg-muted text-muted-foreground`, no shadow, full opacity), because
amber at 50% still reads as a lit, pressable key in Deck. `Button` does this for you; don't
override `disabled:` on a lamp variant.

**Tints.** Use the named washes, not ad-hoc opacities:

| Token | Use |
|---|---|
| `bg-brand-wash` (brand 10%) | selected row, active badge fill |
| `border-brand-edge` (brand 30%) | today/active outline |
| `bg-destructive-wash` / `border-destructive-edge` (5% / 30%) | error callout |
| `bg-success-wash` / `border-success-edge` (10% / 30%) | granted badge; success callout (a topic's Decision) |
| `bg-highlight-yellow` / `bg-highlight-green` | To-do highlighter pens (⌘4 / ⌘5) behind title text; opaque, tuned per theme |

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
  from `lib/page-width.ts`. A centred page passes `PageHeader width="page"` (or `"doc"`) so
  the title, tabs and first card share one left edge when the rail is collapsed. Never wrap
  `PageHeader` in your own width container.
- Document column: `DOC_CONTENT_WIDTH`. Never a bare `max-w-[Npx]` on a page column.
- Rhythm: section gap `mt-8`; panel padding `px-4 py-3` (`p-6` for documents); row padding
  `px-3 py-2`; list stack `gap-0.5`; card stack `gap-3`.
- Control heights: 28 (`xs`), 32 (`sm`), 36 (`default`). Nothing in between.

### Icons

Sparse, mechanical. Icons are imported from `@/components/ui/icons` (never straight from
`lucide-react`) and rendered through `DeckIcon` at 12, 14 (default) or 16px. The barrel *is*
the allowlist: adding a glyph to it is a reviewed design decision.

What is in the barrel, by reason:

- **Functional:** back, chevrons, close, check, overflow `…` (horizontal), play, pause,
  spinner, star toggle, refresh, add, edit, icon-only trash.
- **Functional on icon-only controls**, where the glyph is the control's only face (the
  tooltip carries the words): `Search`, `Eye`/`EyeOff`, `Lock`/`Unlock`, `ChevronsUpDown`,
  `CalendarDays`, `Download`, `RotateCw`, `FileAudio`, `GripVertical`, `BatteryLow`,
  `Link2` (the Prep tab's icon-only "Link previous meeting" button).
- **Menu items:** leading icons on `DropdownMenuItem icon={…}`. Command-palette (⌘K) rows
  count as menu items.
- **Sidebar navigation:** the collapsed rail is icon-only, so each destination's glyph is
  its only wayfinding.
- **AI actions:** `Sparkles` on Ask, Summarize, Generate. It marks "this uses the model".

**Not allowed:** decorative icons on card headers, tabs, text buttons, onboarding steps,
list rows, or empty states (no icon discs). Status is a `StatusDot`, not a glyph. Primitives
in `components/ui/` import from the barrel too.

### Motion

A short opacity fade (`.animate-fade-in`, 140ms). No travel: no `y:`/`x:` entrance offsets.
Continuous animations respect calm motion (specs/0077).

### Focus

Every interactive element shows `focus-visible:ring-2 ring-ring`. Never `focus:outline-none`
without a visible ring.

## 2. Component catalog (`frontend/src/components/ui/`)

| Need | Use | Notes |
|---|---|---|
| Any button with text | `Button` | variants `brand` (primary CTA), `outline` (row action), `ghost`, `link`, `dashed` ("+ Add …"), `destructive` (**confirm inside a dialog only**), `record` (**starts a recording; nothing else**); sizes `xs`/`sm`/`default`/`lg`, plus `icon-sm` (28px square) and `keycap` (24px engraved key cap: `.u-section-label-sm` text, outline edge on `bg-card`; the sidebar's RESTART). A disabled `brand`/`record` goes dark (§Colour) |
| Icon-only button | `IconButton` | `size` `sm` (28) / `md` (32), `variant` `ghost` / `bordered` / `outline` (the icon-only twin of `Button outline`), tooltip from `label` |
| Joined toolbar keys | `ButtonGroup` | `orientation` horizontal (default) / vertical; joins its children's edges and radii. Every segment is `Button variant="outline"` (`size="xs"` in the document toolbars) and an icon segment is `IconButton` / `MenuButton variant="outline"`, so the group has one face. `ButtonGroupSeparator`, `ButtonGroupText` for a divider or a static label |
| Overflow `…` | `MenuButton` | an `IconButton` preset: 14px horizontal glyph, opens a `DropdownMenu` |
| Menu items | `DropdownMenuItem` | `icon` prop; `destructive` variant |
| Page title row | `PageHeader` | `back`, editable `title`, `subtitle` node (not inside the h1), `actions`, `width` `page` / `doc` / `none` (default; aligns the row to the content column) |
| Inline editable text | `EditableText` | the click-to-edit title or field, with the hover pencil |
| View switch | `Tabs` | underline tabs: a hairline rule with a 2px brand underline on the active tab; optional `badge` slot; roving focus built in. Use it whenever the control changes **which view** is on screen (meeting tabs, Settings tabs, Meetings Agenda / Week / Month / List), with or without `TabsContent` (drop `aria-controls` when the panels live elsewhere) |
| Filter | `SegmentedControl` | a boxed key group: recessed `bg-muted` track, the active option a raised `bg-card` cell, `rounded-md`; `size` default (28px) / `compact` (24px) / `lg` (32px, in a row of default-height buttons); one Tab stop (with no active option, the first holds it). Use it when the control **narrows what a view shows** (Action items status / whose, Ask's date scope) |
| Card | `Panel` | `title` (makes it a `<section>` named by the heading), `count` chip, `action` slot, `variant` `solid` / `dashed`, `size` `default` (`px-4 py-3`) / `document` (`p-6`), `titleAs`; header is a `.u-section-label`, no icons. `PanelHeader` is that header row on its own, for a deliberately flat section that sits beside panels (the Prep brief) |
| Inline notice | `Callout` | `tone` `info` (brand wash + edge) / `warn` (brand wash, full brand edge; never red) / `error` (destructive wash + edge, `role="alert"`) / `muted` / `success` (success wash + edge: a settled fact, like a topic timeline's Decision; green there means "decided"); `title`, `action` (primary first), `size` `default` (`px-4 py-3 text-sm`) / `compact` (`px-3 py-2 text-xs`, a status strip under a toolbar), `lead` (a `Spinner` or `StatusDot`, centred on the first line), `actionAlign` `center` (default) / `start` (on the title line, with the body full width underneath, for a tall callout whose body holds rows with their own keys, like the record page's permission notice). Without `title`/`action`/`lead` children render straight into the box. Never override its padding with `className`. `Alert` and `SettingsNote` are thin aliases |
| Empty / loading / error | `EmptyState`, `PageLoading`, `Callout tone="error"` | `EmptyState` `title`, `body`, `action`, `variant` `plain` / `panel` (dashed), `titleAs`; `PageLoading` `label`, `hideLabel`, `fill` `page` (`h-page`) / `pane` (`h-64`). No icon discs |
| Run in progress | `RunStatus` | `label` (the `role="status"` line), `onCancel`, `cancelling`, `cancelLabel`; brand spinner, inside a `Panel` |
| Spinner | `Spinner` | `sm` (14) / `md` (20), inherits colour; `label` makes it a `role="status"`; never a raw `Loader2` or a CSS spinner |
| Progress bar | `Progress` | `h-1.5 rounded-full bg-muted` with a `bg-brand` fill; every download and move bar; `tone` `brand` / `success` / `destructive` |
| Status pill | `Badge` | `tone` `brand` / `success` / `muted` (default) / `destructive` / `outline`; 11px semibold, `rounded-sm`, named washes only |
| Lamp / status / speaker dot | `StatusDot` | `size` 6 / 8 (default) / 10; `tone` `brand` / `record` / `success` / `destructive` / `muted`, or a speaker colour `className`; `label` makes it `role="img"` |
| Person | `Avatar`, `AvatarStack` | `Avatar` `name`, `photo`, `size` 16 / 22 / 24 / 28 / 32 (default) / 56, `colorClass` or `colorKey`, `fallback` (drawn centred inside the avatar's size box, so it takes a photo's slot; a speaker dot in a 16px slot is `StatusDot size={10}`), `title`; `AvatarStack` `people`, `size` 22 / 24, `max` (4px overlap, never clips initials). `initials()` is exported from here (implementation in `lib/initials.ts`) |
| Person picker | `PersonPicker` / `PersonOption` | `PersonPicker` `query`, `onQueryChange`, `onSubmit` (Enter), `inputAction`, `before`, `groups` (`label`, `options`, `emptyMessage`); filtering is the caller's (`matchesPersonQuery`). `PersonOption` `name`, `email`, `suffix`, `lead`, `onSelect`, `selected` |
| List row | `ListRow` | slots `lead`, `title`, `meta`, `trailing`, `menu`; `onClick` makes the main area a button (trailing and menu stay outside it); `selected`, `menuReveal` `always` / `hover`, `ruled`, `hint` (native tooltip on the main area). `LinkRow` `title`, `meta`, `onClick`, `muted`, `hint`: one baseline + chevron, brand on hover |
| Settings | `SettingsSection` (`action` slot on the title line) / `SettingsGroup` / `SettingsRow` / `SettingsNote` | rows put `Button variant="outline" size="sm"` on the right. `SettingsRow` `label`, `description`, `control`, `htmlFor`, `align`, `aside` (a `Badge` on the label line: Active, Default), `muted` (a source present but not in use), children (details below); a list of things in Settings (prompts, calendar sources) is `SettingsRow`s, never hand-rolled rows. The section action has its own column and doesn't stretch the title row, so a section with an action has the same header rhythm as one without. `SettingsGroup` is a `rounded-lg` card; `SettingsNote` is a `Callout` with Callout's own padding |
| Permission | `PermissionRow` + `PermissionStatus` | `state` `granted` / `denied` / `silent` / `checking` (a `StatusDot` and a word, or a spinner); `PermissionRow` `title`, `description`, `statusLabel`, `action`, `variant` `row` / `card`. One shape for settings, onboarding and the modal |
| Selectable card | `OptionCard` / `OptionCardGroup` | `OptionCard` `title`, `description`, `trailing`, `value` (in a group) or `selected` + `onSelect`; radio semantics. `OptionCardGroup` `value`, `onValueChange`, one Tab stop with arrow keys. Model pickers, theme choice |
| Confirm | `ConfirmDialog` | `open`, `onOpenChange`, `title`, `body`, `confirmLabel`, `cancelLabel`, `tone` `destructive` / `brand`, `busy` (spinner, can't be dismissed), `confirmDisabled`, `onConfirm`, `children` for details |
| Date navigation | `DateNavigator` | `label`, `onPrev`, `onNext`, `prevLabel`, `nextLabel`, `onToday`, `showToday`, `leading`, `trailing`, `labelClassName`; Today sits left of the arrows. Today is always the key, in every view (Agenda, Week, Month); the label names the period ("Thu, Nov 12", "This week", "November 2026"), never "Today" |
| Date picker grid | `MiniCalendar` | `value` (`YYYY-MM-DD` or null), `onSelect(iso)`, `today`; one month, Monday-start, always six weeks (stable height); today is amber text with a `brand-edge` ring, the selected day the lit `bg-brand` key, neighbouring-month days engraved; prev/next month `IconButton`s; one Tab stop with arrow-key roving. Lives in a `Popover` (the to-do due-date picker). Never a native `<input type="date">` |
| Form field | `Input`, `Textarea`, `Select`, `Switch`, `Checkbox`, `Label` (`optional` prop) | no native `<input>`/`<select>` in screens. `Input` `size` `default` (36px, 14px) / `lg` (36px, `text-lead`: the Ask question). The buttons in the row under a `lg` input are `size="sm"` |
| Checkbox | `Checkbox` | `checked` / `defaultChecked`, `onCheckedChange`, `label` (names it), `size` `sm` (16) / `default` (18); `onClick` fires first (shift-click ranges). **Checkbox vs Switch:** pick from a list, or an option a dialog's button submits, is a `Checkbox`. A single setting that applies at once is a `Switch` |

**Tabs switch the view; a boxed SegmentedControl filters it; never two underline strips in a
row.** Two underline strips side by side read as two levels of navigation, and an underlined
filter reads as a place you go. A filter bar with two SegmentedControls divides them with a
hairline so they don't read as one five-way control.

Domain components are composed from these and stay where they are. Reuse them before
building another:

- Deck: `ProcessingBadge`, `RecordingBadge`, `StateChip`, `LampDot`, `TapeCounter`,
  `VuMeter`, `TransportKey`.
- `Meetings/DayGroupCard`: one day's card (`Panel` + `ListRow` header), List and Week views.
- `shared/ModelOptionCard`: a model in a picker, an `OptionCard` with its actions beside it.
- `NoteEditor/NoteEditor`: the notes editor. `frame` hands back its toolbar and body so a card can own the title (`Panel title=… action={toolbar}`, the Prep agenda).
- `MeetingDetails/MeetingTitleField`: the editable-title pattern for `PageHeader` (renders
  inside its h1, so no heading of its own).
- `People/PersonActionsMenu`: the one overflow menu for a person.
- `People/InlineEditable`: click-to-edit fields with Escape-revert and placeholders.

## 3. Exceptions

Hardware-faithful parts of the deck (`Transport/` tape counter, VU meter, transport keys,
`ChannelStrip` scale) may use raw sizes and elements. Each exception sits on an allowlist in
`scripts/ui-consistency-allow.txt` with a reason. A one-off exception on a single line uses
`// ui-gate: allow <reason>`. The sanctioned cases:

- Inline text that is itself the trigger (a speaker name, a roster name, a click-to-edit
  description): Button chrome would box it.
- Citation chips and links that flow inside prose.
- Disclosure rows (`aria-expanded`) and other rows with a geometry no primitive has (the
  sidebar's 64px icon column, a month-grid `gridcell`).
- Deck hardware: bevels, lamp glows, the odometer figures.

Anything else is a missing variant: extend the primitive.

## 4. Changing the system

Adding a token, variant or primitive is fine. Do it in `components/ui/` or `globals.css`,
add it to this file in the same change, and run `pnpm shots` then `pnpm shots:diff`.
Changing what an existing token *means* (a lamp colour's role, the radius scale) needs an
ADR.
