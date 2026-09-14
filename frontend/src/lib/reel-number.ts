/**
 * The reel ordinal (specs/0057) — Nixon's archival handle for a meeting.
 *
 * The backend numbers every non-scheduled meeting 1..n oldest-first
 * (`MeetingsRepository::get_meetings_enriched` / `get_reel_number`); the UI renders it
 * like a tape-archive label: `REEL 0412`, zero-padded to four digits so a shelf of
 * labels lines up. Past four digits every digit is kept — the padding is cosmetic, the
 * number is authoritative.
 */
const REEL_PAD = 4;

/** Placeholder for a meeting with no reel ordinal (scheduled placeholders, stale DTOs). */
const REEL_UNKNOWN = 'REEL ——';

/** `412 → "REEL 0412"`. Absent / non-positive / non-finite input → `"REEL ——"`. */
export function formatReelNumber(n?: number | null): string {
  if (n == null || !Number.isFinite(n)) return REEL_UNKNOWN;
  const whole = Math.floor(n);
  if (whole < 1) return REEL_UNKNOWN;
  return `REEL ${String(whole).padStart(REEL_PAD, '0')}`;
}

/**
 * The short spine tag for a log line: `412 → "R0412"`. Where `formatReelNumber` is the
 * full label on the box, this is what fits in a list column. No ordinal → the empty
 * string, so a row's reel cell stays blank rather than printing a placeholder on every
 * line of an un-numbered list.
 */
export function formatReelTag(n?: number | null): string {
  if (n == null || !Number.isFinite(n)) return '';
  const whole = Math.floor(n);
  if (whole < 1) return '';
  return `R${String(whole).padStart(REEL_PAD, '0')}`;
}
