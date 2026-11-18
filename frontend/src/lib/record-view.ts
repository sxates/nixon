/**
 * The record screen's view (owner feedback, v0.18): **Transcript** (the live transcript large,
 * notes / agenda / To-Dos in the right rail), **My Notes** or **Agenda** (the transcript
 * hidden, that one full width as the main column, the Topics + To-Dos sidebar beside it). A
 * per-viewer convenience in `localStorage`, so every access is guarded: a blocked store just
 * means the choice doesn't stick.
 */

export type RecordView = 'transcript' | 'notes' | 'agenda';

export const RECORD_VIEW_KEY = 'nixon.record.view';
export const DEFAULT_RECORD_VIEW: RecordView = 'transcript';

export const isRecordView = (v: unknown): v is RecordView =>
  v === 'transcript' || v === 'notes' || v === 'agenda';

/** Transcript is the only view with the live transcript on screen. */
export const isMainColumnView = (v: RecordView): v is 'notes' | 'agenda' => v !== 'transcript';

export function readRecordView(): RecordView {
  try {
    const v = window.localStorage.getItem(RECORD_VIEW_KEY);
    // 'focus' (v0.18's first cut) was the notes-led layout My Notes now is.
    if (v === 'focus') return 'notes';
    return isRecordView(v) ? v : DEFAULT_RECORD_VIEW;
  } catch {
    return DEFAULT_RECORD_VIEW;
  }
}

export function writeRecordView(view: RecordView): void {
  try {
    window.localStorage.setItem(RECORD_VIEW_KEY, view);
  } catch {
    // Storage blocked (private window, cleared site data): the choice just won't stick.
  }
}

/**
 * The height every top strip of the record screen's panes shares (border included), so the
 * rules under them line up across the screen: the Topics strip, the notes tab strip and the
 * transcript's status strip. A minimum: a strip whose content wraps grows rather than clips.
 */
export const RECORD_STRIP_HEIGHT = 'min-h-11';
