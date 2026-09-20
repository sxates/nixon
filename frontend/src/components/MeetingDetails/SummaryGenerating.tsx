'use client';

/**
 * What the Summary tab shows while a summary is being written (specs/0066).
 *
 * This replaced a bordered circle spinning on its axis — meetily chrome that survived the
 * fork and belonged to no part of Nixon's vocabulary. The deck already says "working" in
 * its own language: the reels. Summarizing is the machine going back over a tape it
 * already has, so it rewinds — backwards, several times transport speed, supply hub
 * leading (see `Reels`). Nothing else in the app rewinds, so the state is unambiguous, and
 * it cannot be mistaken for a take in progress, which spins the other way.
 *
 * No progress count, deliberately. A long transcript is summarized in chunks and the
 * backend does track how many are done, but it attaches that accounting to the *stored
 * result* — there is no progress event while the run is in flight, so the frontend learns
 * the chunk counts only once the summary has already arrived. A "pass 2 of 5" here would
 * therefore be invented. If a progress event is ever added, this is where it belongs.
 */

import { Reels } from '@/components/Transport/Reels';

export function SummaryGenerating() {
  return (
    <div className="flex flex-1 items-center justify-center">
      <div
        className="flex flex-col items-center text-center"
        role="status"
        aria-live="polite"
        aria-label="Writing your summary"
      >
        <Reels state="rewinding" size={44} />
        <p className="u-section-label mt-4 text-[10px] text-engrave">Reading the tape</p>
        <p className="mt-1.5 text-sm text-muted-foreground">Writing your summary…</p>
      </div>
    </div>
  );
}
