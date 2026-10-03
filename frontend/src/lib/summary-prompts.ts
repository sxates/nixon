/** Shared types and pure helpers for saved summary prompts (specs/0079). */

export const MAX_PROMPT_CHARS = 4000;
export const MAX_PROMPT_NAME_CHARS = 60;

export interface SummaryPrompt {
  id: string;
  name: string;
  body: string;
  sort_order: number;
  is_default: boolean;
  extract_action_items: boolean;
  in_library: boolean;
  created_at: string;
  updated_at: string;
}

export type PromptSource = 'custom' | 'meeting' | 'series' | 'default' | 'fallback';

export interface MeetingPromptState {
  source: PromptSource;
  prompt_id: string | null;
  prompt_name: string | null;
  extract_action_items: boolean;
  custom_body: string | null;
  custom_extract_action_items: boolean | null;
  has_series: boolean;
}

/**
 * True when `id` looks like a real SQLite `meetings` row id that per-meeting state
 * may be persisted against. Guards against the two fabricated ids that float around
 * the frontend and must never receive writes (specs/0024 WS3.1, specs/0029 WS4.3):
 * - the sidebar's `'intro-call'` placeholder, and
 * - TranscriptContext's IndexedDB id, `meeting-<Date.now()>` (an all-digit suffix —
 *   real rows are `meeting-<uuid>`, which always contains hex letters/hyphens).
 */
export function isPersistableMeetingId(id: string | null | undefined): id is string {
  if (!id || !id.trim()) return false;
  if (id === 'intro-call') return false;
  if (/^meeting-\d+$/.test(id)) return false;
  return true;
}

/** Short label for where a meeting's effective prompt comes from. */
export function promptSourceLabel(state: MeetingPromptState | null): string {
  if (!state) return 'Default';
  switch (state.source) {
    case 'custom':
      return 'Custom (this meeting)';
    case 'meeting':
      return state.prompt_name ?? 'Prompt';
    case 'series':
      return `${state.prompt_name} (series)`;
    case 'default':
      return `${state.prompt_name ?? 'Default'}`;
    default:
      return 'Default';
  }
}

/**
 * Confirm-before-regenerate: picking a different prompt (or leaving a one-off
 * custom prompt) on a meeting that already has a summary must ask first.
 */
export function shouldConfirmPromptChange(
  nextPromptId: string | null,
  state: MeetingPromptState | null,
  hasExistingSummary: boolean,
): boolean {
  if (!hasExistingSummary) return false;
  return nextPromptId !== state?.prompt_id || state?.source === 'custom';
}
