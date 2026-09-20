/**
 * The sidebar's row geometry, in one place (specs/0069 W1).
 *
 * Expanded and collapsed used to be two layouts: the rail centred a 16px glyph in a 64px
 * column on `h-10` rows, while the panel put a 14px glyph at x=27px on `h-8` rows — so
 * opening the sidebar moved every icon left and up and shrank it. They now share one
 * contract, and the only difference between the states is the panel's width and whether
 * labels render.
 *
 * The row itself has NO padding. The icon column is exactly the collapsed rail's width, so
 * a glyph sits at x=32px either way; a `pl-*` here would push the collapsed glyph off
 * centre, and a `pr-*` would make a collapsed row wider than the rail. The right inset
 * belongs to [`SIDEBAR_LABEL`], which exists only when expanded.
 */

/** Height, alignment and full width shared by every sidebar row. No padding — see above. */
export const SIDEBAR_ROW = 'flex h-10 w-full items-center';

/**
 * The leading-icon column: the collapsed rail's full width (`w-16` = 64px), so every
 * leading element — a glyph, the reel mark, a lamp, the DEV badge — is centred on the same
 * x in both states.
 */
export const SIDEBAR_ICON_SLOT = 'flex w-16 flex-none items-center justify-center';

/** The label beside the slot. Rendered only when expanded; carries the row's right inset. */
export const SIDEBAR_LABEL = 'u-section-label min-w-0 flex-1 truncate pr-3.5 text-left';

/** Glyph size, in px: the rail's 16, now used in both states. */
export const SIDEBAR_GLYPH = 16;
