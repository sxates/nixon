# ADR-0014: Audio lifecycle — capture everything, compress what is kept, one retention policy

Status: Proposed
Date: 2026-09-22

## Context

A meeting folder holds three audio files: the mixed `audio.mp4` (AAC 192 kbps), which speech
recognition reads, and two 16 kHz PCM channel files, `mic.wav` and `system.wav`, which speaker
identification reads. Together they come to about 317 MB per hour, and the channel WAVs are
73% of that.

The "Delete audio recordings" setting was stored as two preferences, and one of them
(`auto_save`) was also the switch for whether the mix was captured at all. That caused three
problems:

- "Immediately" skipped the mix but still wrote both channel WAVs, and nothing ever deleted
  them.
- Deferred processing had no mix to transcribe.
- Deletion ran on a timer that didn't know whether processing had finished. Processing is
  orchestrated from the UI, and its outcome was never recorded.

## Decision

1. **Capture doesn't depend on retention.** Every recording writes the mix and both channels.
   The retention setting only decides what happens after processing.
2. **Rust keeps a per-meeting lifecycle state.** `meetings.audio_state` is NULL (pending),
   `processed`, `failed` or `purged`. Alongside it, `meetings.speakers_identified_at` records
   the last successful speaker identification. Rust writes both at the points where
   processing actually ends: diarization's outcome, the deferred backlog clearing its marker,
   and import completion. A startup reconcile finishes any meeting whose processing was
   interrupted by quitting the app.
3. **One pure policy function decides keep, compress or delete.** Its input is the policy
   (`AfterProcessing`, `Days(n)` or `Forever`), the meeting's facts and the current time. It
   runs when processing finishes, at startup, hourly, and immediately after the user changes
   the setting. The dry-run preview in Settings calls the same function, so the preview and
   the actual deletion can't disagree.
4. **Pending audio is never deleted.** Audio whose processing hasn't finished is kept
   regardless of age or policy. Audio whose speaker identification failed is kept for a
   bounded grace period so it can be retried.
5. **Kept audio is compressed once processing finishes.** Each channel is re-encoded to Opus
   in an Ogg container (`mic.opus`, `system.opus`) with the bundled ffmpeg's `libopus` at
   **24 kbps** (`-application audio`), the lowest rung that passed the diarization DER gate
   (see "Codec gate" below). The encoded file is verified by decoding it and comparing
   durations before the WAV is removed. New recordings write the mix at **64 kbps** AAC-LC,
   which passed the WER gate. Every reader decodes through
   `audio::decoder::decode_audio_file`, which falls back to ffmpeg for Opus.
6. **Background jobs that touch a meeting folder go through the per-meeting folder lease**
   (see the folder-lease ADR). This covers the compressor and the sweep.

## Codec gate (measured 2026-09-23)

The eval harnesses round-trip their input through the candidate codec with the bundled ffmpeg
(`tests/eval_codec/`, `NIXON_EVAL_CODEC` / `NIXON_WER_CODEC`) and score the decoded audio.
The corpus is three labelled local meeting recordings, about 3.6 hours in total. It already
went through one AAC pass, so these numbers measure a second lossy pass, which is harsher than
production.

- **Channels, DER.** Acceptance: macro DER within +0.5 pt of baseline, and every pinned
  per-file ceiling holds. At the shipped audio-seed path, Opus 24 kbps scored a macro DER of
  7.93% against a baseline of 8.07% (−0.14 pt), and every pin passed. The shipped Auto sweep
  row moved from 8.19% to 8.34% (+0.15 pt). The ladder stopped at its first rung, so 32 kbps,
  48 kbps and FLAC were not needed. Opus 24 kbps is about 8% of the 16 kHz PCM size, around
  9.4 MB per channel-hour.
- **Mix, WER.** Acceptance: within +1.0 pt of baseline for the default engine (Parakeet). The
  pooled whole-file WER against Zoom's own transcript was 11.3% for the baseline, 10.6% for
  AAC 64 kbps and 10.7% for AAC 192 kbps (today's rate). Both round-trips beat the baseline
  because of the decode path, not the codec. The comparison that matters is 64k against 192k,
  and the difference is negligible.

## Consequences

- Kept audio falls from ~317 MB/h to about 48 MB/h (mix ~29 MB/h, two Opus channels
  ~19 MB/h).
- The channel files are lossy after processing. Running speaker identification again works
  on them, and the eval harness quantifies the quality cost.
- When the migration first runs, installs that had "Immediately" selected lose the leftover
  channel WAVs that setting should never have kept.
- Voiceprints are unaffected. They are 192-dimensional embedding vectors (ADR-0007,
  ADR-0011), contain no audio, and speaker rename, reassign and merge work without audio.
- `auto_save` and `retention_days` remain in the preferences file only so older files still
  deserialize. Nothing reads them.
- Any new consumer of the channel files must resolve them through `system_channel_path` /
  `mic_channel_path`, because the file may be `.wav` or `.opus`.

## Alternatives considered

- **Compress during capture.** Rejected. WAV can be repaired after a crash and is what the
  pipeline reads.
- **FLAC for the channels.** Lossless, but still about 60% of the WAV size. It's kept as the
  fallback if Opus fails the DER gate.
- **The UI marks processing as finished.** Rejected. A crash between steps leaves the meeting
  stranded, and the UI isn't the only orchestrator: import and the backlog also run
  processing.
- **Treat a meeting as processed once it has speaker rows.** Rejected. Live diarization
  creates speaker rows while the meeting is still recording.
