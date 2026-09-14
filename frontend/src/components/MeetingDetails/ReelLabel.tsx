import { formatReelNumber } from '@/lib/reel-number';

/**
 * The reel label (specs/0057) — the typed paper card that sits to the right of the
 * meeting-details title, styled like the sticker on an archived tape: typewriter ink on
 * paper, a red rule under the header, and a two-column ledger of the recording's facts.
 *
 * Everything is pre-formatted by the caller (the header owns the locale/duration
 * formatting); this component only lays the card out.
 */
export interface ReelLabelProps {
  /** Archival ordinal from the backend; `null`/absent prints the blank `REEL ——` handle. */
  reelNumber?: number | null;
  /** Local date, e.g. "Tue Jun 24". */
  date: string;
  /** Local start time, e.g. "10:00". */
  time: string;
  /** Recording length, e.g. "00:42:18". */
  length: string;
  /** Where the audio came from, e.g. "Zoom" / "System audio". */
  source: string;
  /** Distinct speakers, when known. */
  voices?: number;
  /** Storage deck / folder marker, when known. */
  deck?: string;
  /** Outlined status chips, e.g. ["Summarized", "3 tasks"]. */
  tags?: string[];
}

/** One `LABEL  value` line of the ledger. `null` values drop the row entirely. */
function ReelRow({ label, value }: { label: string; value: string | null }) {
  if (value == null || value === '') return null;
  return (
    <div className="contents" data-testid={`reel-row-${label}`}>
      <dt className="text-engrave">{label}</dt>
      <dd className="truncate font-medium">{value}</dd>
    </div>
  );
}

export function ReelLabel({
  reelNumber,
  date,
  time,
  length,
  source,
  voices,
  deck,
  tags,
}: ReelLabelProps) {
  return (
    <aside
      aria-hidden="true"
      className="u-typed w-[188px] shrink-0 rounded-[2px] border border-border bg-paper px-3.5 py-3 uppercase tracking-[0.02em] text-paper-ink shadow-[0_1px_2px_rgba(40,30,20,0.08)]"
    >
      <p data-testid="reel-label-header" className="text-[12px] font-bold">
        {formatReelNumber(reelNumber)} · NIXON
      </p>
      {/* The red rule: pure decoration, never announced. */}
      <span aria-hidden="true" className="mb-2 mt-1.5 block h-px bg-record/55" />
      <dl className="grid grid-cols-[auto_1fr] gap-x-2.5 gap-y-0.5 text-[11px] leading-[1.55]">
        <ReelRow label="DATE" value={date} />
        <ReelRow label="TIME" value={time} />
        <ReelRow label="LENGTH" value={length} />
        <ReelRow label="SOURCE" value={source} />
        <ReelRow label="VOICES" value={voices == null ? null : String(voices)} />
        <ReelRow label="DECK" value={deck ?? null} />
      </dl>
      {!!tags?.length && (
        <ul className="mt-2.5 flex flex-wrap gap-1">
          {tags.map((tag) => (
            <li
              key={tag}
              className="rounded-[2px] border border-border px-1.5 py-px text-[10px] text-engrave"
            >
              {tag}
            </li>
          ))}
        </ul>
      )}
    </aside>
  );
}
