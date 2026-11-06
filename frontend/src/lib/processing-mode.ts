/**
 * Low-power-mode display helpers (spec low-power-mode, §§3,5).
 *
 * The session bookkeeping that used to live here (whether the session started or was ever
 * deferred, and the stop-time `stopAction`) moved to Rust with the wrap-up (specs/0083:
 * `audio/live_toggle.rs` and `audio/wrap_up/stop_action.rs`, which carry the test tables).
 */

/**
 * Effective "is live transcription on" value for display purposes, once a
 * `useProcessingMode()` consumer wants to show something before the first
 * `processing-mode-changed` event has arrived. Prefer the live event value;
 * fall back to a caller-supplied stored preference until it does.
 */
export function effectiveLiveTranscription(
  eventValue: boolean | null,
  fallbackPref: boolean,
): boolean {
  return eventValue ?? fallbackPref;
}

/** Compact mode-chip label + battery-glyph decision (RecordingHeader). */
export interface ModeChipDisplay {
  label: 'Pause transcript' | 'Resume transcript';
  showBatteryGlyph: boolean;
}

/**
 * What the chip's click will DO — not what mode the session is in.
 *
 * It used to read `Live` / `Deferred`, i.e. the current state, while clicking *left* that
 * state. The owner read the word "Live" as "make it live", pressed it mid-meeting, and the
 * transcript stopped (2026-09-21; specs/0071). That is the only reading a reasonable person
 * takes from a button in a row where its two neighbours — Participants and the template
 * picker — both do the thing written on them.
 *
 * The state the label used to carry now lives on the transport rail's status line, which
 * already owns the session's state vocabulary ("On the reel" / "On hold" / "Mic muted") and
 * is on screen on every route.
 */
export function modeChipDisplay(
  live: boolean,
  onBattery: boolean | null,
): ModeChipDisplay {
  return {
    label: live ? 'Pause transcript' : 'Resume transcript',
    // Still the chip's business: it explains WHY the transcript is paused.
    showBatteryGlyph: !live && onBattery === true,
  };
}

/**
 * Which record-only empty-state variant TranscriptPanel should show, if any.
 * `null` means the normal (live) transcript view should render instead.
 *
 * - `'low-power-battery'`: this session is deferred specifically because it's
 *   running on battery (Low Power Mode) — offer a per-meeting "go live" button.
 * - `'live-transcription-off'`: deferred for any other reason (the global
 *   live-transcription setting is off) — the existing quiet explanation, no
 *   per-meeting override offered since there's nothing battery-specific to undo.
 */
export type EmptyStateVariant = 'low-power-battery' | 'live-transcription-off';

export function transcriptEmptyStateVariant(
  liveTranscriptionEnabled: boolean,
  hasTranscripts: boolean,
  onBattery: boolean | null,
): EmptyStateVariant | null {
  if (liveTranscriptionEnabled || hasTranscripts) return null;
  return onBattery === true ? 'low-power-battery' : 'live-transcription-off';
}

/**
 * A finished recording that has audio on disk but no transcript yet — i.e. recorded in
 * low-power/deferred mode and not processed. Drives the transcript empty state so it reads
 * "not processed yet" instead of the generic welcome copy (spec 0045 WS4).
 */
export function isUnprocessedRecording(input: {
  hasTranscripts: boolean;
  isDeferred: boolean;
  audioAvailable: boolean;
}): boolean {
  return !input.hasTranscripts && input.isDeferred && input.audioAvailable;
}
