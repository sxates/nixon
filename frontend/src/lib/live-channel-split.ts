/** specs/0055 — splitting a live transcript row at owner<->remote handoffs.
 *
 *  A transcript row is one VAD segment, and real conversational handoffs are
 *  faster than the VAD's redemption window, so 32% of rows in a measured real
 *  meeting contain both the owner's speech and a remote speaker's. The row's
 *  single `channel` tag is a whole-row RMS vote, so it necessarily mislabels one
 *  side of those rows — the owner's opening words take the previous speaker's
 *  name, or the next speaker's opening words show as "You".
 *
 *  The backend now also sends `channel_runs`: the same evidence at 600 ms window
 *  resolution, tiling the row's span. Here we cut the row where the owner/remote
 *  side changes and apportion the text across the parts by duration, snapping to
 *  whitespace. This is render-time only — the stored row is untouched, and the
 *  authoritative offline pass at stop still relabels everything.
 *
 *  Apportioning is proportional, not word-exact: the live `transcript-update`
 *  event carries no per-word timings (only the saved rows do). Word-exact cuts
 *  are Stage 2's job, in the offline pass.
 */

/** Window-resolution capture-channel evidence for one segment span, from the
 *  backend's `channel_runs_for_span` (compact wire shape). Times are
 *  recording-relative seconds. */
export interface ChannelRun {
  s: number;
  e: number;
  c: 'microphone' | 'system' | 'mixed';
}

/** One renderable slice of a straddling row. */
export interface LivePart {
  text: string;
  channel: ChannelRun['c'];
  start: number;
  end: number;
}

/** The owner/remote axis. Only `microphone` is the owner; `system` and `mixed`
 *  are both "not provably the owner" and are never labeled "You" (specs/0043
 *  W1.3 — a wrong "You" is worse than no name). */
const isOwner = (run: ChannelRun): boolean => run.c === 'microphone';

/** Minimum owner stretch (seconds) that may be carved out as a "You" part.
 *
 *  Mirrors `MIN_SPLIT_PART_SECS` in the offline splitter (`diarization/split.rs`),
 *  so live and offline agree on what counts as a confident turn.
 *
 *  Why it exists, and why only on the owner side: on speakers the mic path is
 *  EBU R128-normalized, so the remote voice bleeding into it is boosted to speech
 *  loudness. During an inter-phrase dip the system track can fall below
 *  `CHANNEL_BLEED_SYSTEM_RMS` while that echo still clears the 3x ratio, and a
 *  single 600 ms window classifies `Microphone`. The whole-row vote used to
 *  absorb such blips; splitting would otherwise turn one into a "You" clip in the
 *  middle of someone else's sentence — the regression specs/0043 W1.3 and 0047 W2
 *  exist to prevent. The asymmetry is deliberate: a wrong "You" is worse than a
 *  missing one, so a brief owner run is demoted, while a brief remote run is not.
 *
 *  Measured on the real capture: this cuts remote speech mislabeled "You" from
 *  12.0s to 8.2s and total source error from 4.74% to 3.68%, on a broad plateau
 *  (0.7-1.0s all land within 0.25pt) rather than a tuned spike. */
const MIN_OWNER_PART_SECS = 0.75;

/** Split a live row at its owner<->remote changes.
 *
 *  Returns `null` — meaning "render exactly as before" — when the row carries no
 *  runs, sits entirely on one side of the owner/remote line, has too few words to
 *  divide, or when a side is too brief to be apportioned even one word. Callers
 *  therefore only need a new render path for the genuinely straddling rows.
 */
export function splitLiveRowByChannel(
  text: string,
  start: number,
  end: number,
  runs: ChannelRun[] | undefined,
): LivePart[] | null {
  const span = end - start;
  if (!runs || runs.length === 0 || span <= 0) return null;

  const words = text.trim().split(/\s+/).filter(Boolean);
  if (words.length < 2) return null;

  // Merge adjacent runs that fall on the same side of the owner/remote line.
  const groups: Array<{ start: number; end: number; channel: ChannelRun['c'] }> = [];
  for (const run of runs) {
    const last = groups[groups.length - 1];
    if (last && isOwner(run) === (last.channel === 'microphone')) {
      last.end = Math.max(last.end, run.e);
    } else {
      groups.push({ start: run.s, end: run.e, channel: run.c });
    }
  }
  // Demote owner stretches too brief to be a confident turn (see
  // MIN_OWNER_PART_SECS), then re-merge — a blip between two remote stretches
  // leaves one remote group, so the row stays whole.
  const guarded: typeof groups = [];
  for (const group of groups) {
    const brief =
      group.channel === 'microphone' && group.end - group.start < MIN_OWNER_PART_SECS;
    const channel = brief ? 'mixed' : group.channel;
    const last = guarded[guarded.length - 1];
    if (last && (last.channel === 'microphone') === (channel === 'microphone')) {
      last.end = Math.max(last.end, group.end);
    } else {
      guarded.push({ ...group, channel });
    }
  }
  groups.length = 0;
  groups.push(...guarded);
  if (groups.length < 2) return null;

  // Apportion words by cumulative duration so the counts always sum to the whole.
  const parts: LivePart[] = [];
  let taken = 0;
  for (let i = 0; i < groups.length; i++) {
    const group = groups[i];
    const boundary =
      i === groups.length - 1
        ? words.length
        : Math.round((words.length * (group.end - start)) / span);
    const count = boundary - taken;
    // A side too brief to earn a word isn't a split we can render honestly.
    if (count < 1) return null;
    parts.push({
      text: words.slice(taken, boundary).join(' '),
      channel: group.channel,
      start: group.start,
      end: group.end,
    });
    taken = boundary;
  }
  return parts;
}

/** The fields of a live transcript row this module needs. Structural, so the
 *  panel can pass its `Transcript` objects straight in. */
export interface SplittableTranscript {
  id: string;
  text: string;
  audio_start_time?: number;
  audio_end_time?: number;
  confidence?: number;
  speaker?: string | null;
  speaker_name?: string | null;
  channel_runs?: ChannelRun[];
}

/** One renderable transcript row for the virtualized live view. */
export interface RenderSegment {
  id: string;
  timestamp: number;
  endTime?: number;
  text: string;
  confidence?: number;
  speaker?: string | null;
  speakerName?: string | null;
}

/** The only display name this module is entitled to reinterpret. A live row shows
 *  "You" solely because its whole-row `channel` tag said `microphone` — never
 *  because diarization said so (the live pass can't emit `local`, specs/0043
 *  W1.3), so a "You" here is exactly the label the runs supersede. Any other name
 *  came from a live diarization pass and is stable-once-shown. */
const CHANNEL_DERIVED_LABEL = 'You';

/** Color key for a part, matching `liveColorKey` in TranscriptContext: the owner
 *  takes the pinned `local` slot, an unlabeled part takes none. */
const colorKeyFor = (channel: ChannelRun['c']): string | null =>
  channel === 'microphone' ? 'local' : null;

/** Expand live transcript rows into the segments the panel renders, splitting any
 *  row that straddles an owner<->remote handoff (specs/0055).
 *
 *  Render-time only: the rows in `TranscriptContext` state — and therefore the
 *  save payload — are untouched, so this changes nothing about what is persisted.
 *  The authoritative offline pass at stop relabels the stored rows separately.
 */
export function expandTranscriptSegments(
  transcripts: SplittableTranscript[],
): RenderSegment[] {
  const segments: RenderSegment[] = [];
  for (const t of transcripts) {
    const whole: RenderSegment = {
      id: t.id,
      timestamp: t.audio_start_time ?? 0,
      endTime: t.audio_end_time,
      text: t.text,
      confidence: t.confidence,
      speaker: t.speaker,
      speakerName: t.speaker_name,
    };
    const ourLabel = !t.speaker_name || t.speaker_name === CHANNEL_DERIVED_LABEL;
    const parts = ourLabel
      ? splitLiveRowByChannel(
          t.text,
          t.audio_start_time ?? 0,
          t.audio_end_time ?? 0,
          t.channel_runs,
        )
      : null;
    if (!parts) {
      segments.push(whole);
      continue;
    }
    parts.forEach((part, i) => {
      const owner = part.channel === 'microphone';
      segments.push({
        id: `${t.id}#${i}`,
        timestamp: part.start,
        endTime: part.end,
        text: part.text,
        confidence: t.confidence,
        speaker: colorKeyFor(part.channel),
        speakerName: owner ? CHANNEL_DERIVED_LABEL : null,
      });
    });
  }
  return segments;
}
