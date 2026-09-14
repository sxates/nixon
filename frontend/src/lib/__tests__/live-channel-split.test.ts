import { describe, it, expect } from 'vitest';
import { splitLiveRowByChannel, type ChannelRun } from '@/lib/live-channel-split';

// specs/0055 — 32% of real transcript rows straddle an owner<->remote handoff, and
// the single `channel` tag necessarily mislabels one side of those. These lock the
// live render-time split on the window-resolution runs.

const mic = (s: number, e: number): ChannelRun => ({ s, e, c: 'microphone' });
const remote = (s: number, e: number): ChannelRun => ({ s, e, c: 'mixed' });

describe('splitLiveRowByChannel', () => {
  it('splits an owner-to-remote row into two parts, the first labeled You', () => {
    // 10 s row: 5 s of the owner, then 5 s of the remote speaker.
    const parts = splitLiveRowByChannel(
      'i think we should ship it yeah that works for me',
      0,
      10,
      [mic(0, 5), remote(5, 10)],
    );
    expect(parts).not.toBeNull();
    expect(parts).toHaveLength(2);
    expect(parts![0].channel).toBe('microphone');
    expect(parts![1].channel).toBe('mixed');
    expect(parts![0].text).toBe('i think we should ship it');
    expect(parts![1].text).toBe('yeah that works for me');
  });

  it('returns null when every run is on the same side of the owner/remote line', () => {
    // 'system' and 'mixed' are both "not the owner" — no split to make.
    expect(
      splitLiveRowByChannel('all of this is remote', 0, 6, [
        remote(0, 3),
        { s: 3, e: 6, c: 'system' },
      ]),
    ).toBeNull();
    expect(splitLiveRowByChannel('all mine', 0, 4, [mic(0, 4)])).toBeNull();
  });

  it('returns null when the row carries no channel runs', () => {
    // Legacy / no classified windows: render exactly as before specs/0055.
    expect(splitLiveRowByChannel('no evidence', 0, 4, [])).toBeNull();
    expect(splitLiveRowByChannel('no evidence', 0, 4, undefined)).toBeNull();
  });

  it('never cuts a word in half', () => {
    // The proportional boundary lands mid-word; it must snap to whitespace.
    const parts = splitLiveRowByChannel('alpha bravo charlie delta', 0, 10, [
      mic(0, 4.7),
      remote(4.7, 10),
    ]);
    expect(parts).not.toBeNull();
    const rejoined = parts!.map((p) => p.text).join(' ');
    expect(rejoined).toBe('alpha bravo charlie delta');
    for (const part of parts!) {
      expect(part.text).toBe(part.text.trim());
      expect(part.text.length).toBeGreaterThan(0);
    }
  });

  it('does not emit an empty part when one side is too brief to take a word', () => {
    // A 0.1 s owner blip inside a 10 s remote row: proportional apportioning would
    // give it no words, so there is nothing to split off.
    expect(
      splitLiveRowByChannel('one two three four five', 0, 10, [
        mic(0, 0.1),
        remote(0.1, 10),
      ]),
    ).toBeNull();
  });

  it('does not carve a You part out of a brief mic blip inside a remote row', () => {
    // Code review, specs/0055: on speakers the R128-normalized mic carries the
    // remote voice, and during an inter-phrase dip a single 600 ms window can
    // classify Microphone. The whole-row vote used to absorb that; splitting must
    // not turn it into a "You" clip mid-sentence — the exact regression 0043 W1.3
    // and 0047 W2 exist to prevent. A wrong "You" is worse than a missing one.
    expect(
      splitLiveRowByChannel('one two three four five six seven eight nine ten', 0, 10, [
        remote(0, 4),
        mic(4, 4.5),
        remote(4.5, 10),
      ]),
    ).toBeNull();
  });

  it('keeps an owner part that clears the minimum', () => {
    // The guard must not swallow a real short reply: 1.0s of owner speech is a
    // genuine turn and still splits out.
    const parts = splitLiveRowByChannel(
      'one two three four five six seven eight nine ten',
      0,
      10,
      [remote(0, 4), mic(4, 5), remote(5, 10)],
    );
    expect(parts).not.toBeNull();
    expect(parts).toHaveLength(3);
    expect(parts![1].channel).toBe('microphone');
  });

  it('carries each part its own time span', () => {
    const parts = splitLiveRowByChannel('alpha bravo charlie delta', 100, 110, [
      mic(100, 105),
      remote(105, 110),
    ]);
    expect(parts).not.toBeNull();
    expect(parts![0].start).toBe(100);
    expect(parts![0].end).toBe(105);
    expect(parts![1].start).toBe(105);
    expect(parts![1].end).toBe(110);
  });
});

// --- render-time expansion used by TranscriptPanel ---

import { expandTranscriptSegments, type SplittableTranscript } from '@/lib/live-channel-split';

const row = (over: Partial<SplittableTranscript> = {}): SplittableTranscript => ({
  id: 'row-1',
  text: 'i think we should ship it yeah that works for me',
  audio_start_time: 0,
  audio_end_time: 10,
  confidence: 0.9,
  speaker: null,
  speaker_name: null,
  channel_runs: [mic(0, 5), remote(5, 10)],
  ...over,
});

describe('expandTranscriptSegments', () => {
  it('leaves a row that does not straddle a handoff as a single segment', () => {
    const out = expandTranscriptSegments([row({ channel_runs: [mic(0, 10)] })]);
    expect(out).toHaveLength(1);
    expect(out[0].id).toBe('row-1');
    expect(out[0].text).toBe('i think we should ship it yeah that works for me');
  });

  it('splits a mic-tagged straddling row so only the owner part says You', () => {
    // The specs/0055 symptom: the whole row was hard-labeled "You", carrying the
    // next speaker's opening words with it.
    const out = expandTranscriptSegments([row({ speaker_name: 'You', speaker: 'You' })]);
    expect(out).toHaveLength(2);
    expect(out[0].speakerName).toBe('You');
    expect(out[0].text).toBe('i think we should ship it');
    expect(out[1].speakerName).toBeNull();
    expect(out[1].text).toBe('yeah that works for me');
  });

  it('gives the owner part a You label on a row that had no label at all', () => {
    // The mirror symptom: the owner's opening words sat in an unlabeled row.
    const out = expandTranscriptSegments([row()]);
    expect(out).toHaveLength(2);
    expect(out[0].speakerName).toBe('You');
    expect(out[1].speakerName).toBeNull();
  });

  it('never splits a row that live diarization already labeled', () => {
    // Live labels are stable-once-shown; a real diarization name is not ours to
    // reinterpret (only the channel-tag-derived "You" is).
    const out = expandTranscriptSegments([row({ speaker_name: 'Speaker 2', speaker: 'spk_1' })]);
    expect(out).toHaveLength(1);
    expect(out[0].speakerName).toBe('Speaker 2');
  });

  it('gives every expanded part a distinct id', () => {
    const ids = expandTranscriptSegments([row()]).map((s) => s.id);
    expect(new Set(ids).size).toBe(ids.length);
  });
});
