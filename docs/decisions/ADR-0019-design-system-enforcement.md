# ADR-0019: One design system, enforced by a ratcheting gate

Status: Accepted
Date: 2026-11-11

## Context

The deck look (specs/0057) was applied screen by screen. A four-area UX audit
(2026-11-11) found the primitives in `components/ui/` largely bypassed:

- 192 hand-rolled `<button>` in 87 files.
- 134 arbitrary font sizes, with no `fontSize` scale in the Tailwind config.
- One corner radius spelled seven ways.
- Three segmented-control styles, three empty-state styles, and four renderings of a card
  header.
- Eight hand-built progress bars and seven copies of the same confirm dialog.

Visible bugs came out of the drift: misaligned headers, a red "Generate Summary" CTA, and
onboarding buttons that switch colour between steps. Review alone hasn't held the line,
because every new screen copies the nearest neighbour.

## Decision

1. **`docs/design-system.md` is the contract.** It holds the tokens, the type scale and
   the component catalog. The `ux-designer` agent reviews against it.
2. **Screens compose primitives.** A missing variant is added to the primitive, never
   re-created beside it.
3. **A gate, `scripts/check-ui-consistency.sh`, runs in `/check` and CI.** It counts
   violations of mechanically checkable rules:
   - raw `<button>`, `<input>` or `<select>` outside `components/ui/`
   - `text-[Npx]`
   - `rounded-[…]` / `rounded-xl|2xl`
   - `.u-section-label` combined with a size override
   - arbitrary `shadow-[…]`
   - `lucide-react` imports outside `components/ui/` (the approved glyphs are re-exported
     from `components/ui/icons.ts`, which is the allowlist)
   - framer-motion entrance travel
   - `bg-brand/90`-style hand-rolled hover states

   Existing violations are grandfathered as a **per-rule budget that only goes down**. This
   uses the same ratchet as the file-size gate (specs/0065): `--update` lowers the budget,
   nothing raises it. Hardware-faithful deck parts are allowlisted by file with a reason.
4. **Icon policy:**
   - functional glyphs only
   - dropdown menu items may keep leading icons
   - AI actions keep `Sparkles` as the "uses the model" marker (owner decision, 2026-11-11)

## Consequences

- New UI can't add drift, and every refactor shows up as a lower budget.
- Some screens shift by a pixel or two as their spellings collapse to the scale. Each
  migration wave is checked with `pnpm shots:diff`.
- Grep rules can't see intent. Visual consistency still needs the UX review, but the review
  no longer has to catch mechanical drift.
