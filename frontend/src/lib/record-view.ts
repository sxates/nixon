/**
 * The record screen's view (owner feedback, v0.18): **Transcript** (the live transcript large,
 * notes in the right rail) or **Focus** (the transcript hidden, the notes column full width
 * with a To-Do rail beside it). A per-viewer convenience in `localStorage`, so every access
 * is guarded: a blocked store just means the choice doesn't stick.
 */

export type RecordView = 'transcript' | 'focus';

export const RECORD_VIEW_KEY = 'nixon.record.view';
export const DEFAULT_RECORD_VIEW: RecordView = 'transcript';

export const isRecordView = (v: unknown): v is RecordView => v === 'transcript' || v === 'focus';

export function readRecordView(): RecordView {
  try {
    const v = window.localStorage.getItem(RECORD_VIEW_KEY);
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
