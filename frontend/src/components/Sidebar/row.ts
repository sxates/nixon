/**
 * The sidebar's row geometry, in one place (owner feedback 2026-09-19).
 *
 * Every row in the expanded panel — the hamburger/wordmark bar, the nav destinations,
 * Import audio, Update, Queue and Settings — had grown its own left inset (`pl-3`, `pl-5`,
 * `px-3`) and its own leading element (a 14px glyph, an 8px lamp, an 18px hamburger). The
 * icons landed in three different columns and the labels in four.
 *
 * Two constants fix that: rows share [`SIDEBAR_ROW`] for their padding and gap, and every
 * leading element — glyph or lamp — sits inside an [`SIDEBAR_ICON_SLOT`] box of the glyph's
 * own width. A lamp is then centred in the same column a glyph occupies, and because the
 * slot is a fixed width, every label starts at the same x no matter what precedes it.
 */

/** Padding + gap shared by every full-width row in the expanded sidebar. */
export const SIDEBAR_ROW = 'flex items-center gap-2.5 pl-5 pr-3.5';

/**
 * The leading-icon column: exactly as wide as a `DeckIcon` (14px, the deck chrome size), so a
 * narrower mark (the 8px lamp) centres in it instead of dragging its label leftwards.
 */
export const SIDEBAR_ICON_SLOT = 'flex w-3.5 flex-none items-center justify-center';
