/**
 * Ask-AI (specs/0035) — IPC types and pure helpers for the /ask page.
 *
 * Mirrors the Rust `aggregation` module's serde contracts exactly
 * (`rename_all = "camelCase"` on every payload):
 *   - commands: `api_ask_ai_run`, `api_cancel_ask_ai`
 *   - events:   `ask-ai-progress`, `ask-ai-complete`, `ask-ai-error`
 *
 * Keeping the types + helpers here (not in the page) lets the Vitest layer
 * exercise the scope/Enter-key logic without a Tauri shell.
 *
 * specs/0038 WS2 adds a thin persistence layer on top of the 0035 engine: every
 * completed run is auto-recorded to history (WS2.a), and a question+scope can be
 * "starred" for later re-run (WS2.b). The `invoke` wrappers for those live at the
 * bottom of this file; the engine/run flow is unchanged.
 */

import { invoke } from '@tauri-apps/api/core';

/** Which meetings a question may draw from. Omitted fields mean "all". */
export interface AggregationScope {
  /** Explicit meeting set — bypasses relevance ranking. */
  meetingIds?: string[];
  /**
   * ISO-8601 UTC instant lower bound on `meetings.created_at`, INCLUSIVE —
   * local midnight at the start of the first selected day.
   */
  dateFrom?: string;
  /**
   * ISO-8601 UTC instant upper bound, EXCLUSIVE — local midnight at the start
   * of the day AFTER the last selected day.
   */
  dateTo?: string;
  /** `people.id` — roster OR speaker membership. */
  personId?: string;
}

/** One meeting the engine numbered — `sources[i]` ↔ citation marker `[M{i+1}]`. */
export interface SourceMeeting {
  meetingId: string;
  title: string;
  createdAt: string; // ISO-8601 UTC
  cited: boolean;
}

export type AskAiStage = 'gathering' | 'mapping' | 'reducing';

export interface AskAiProgressPayload {
  runId: string;
  stage: AskAiStage;
  /** 1-based doc index during "mapping"; 0 otherwise. */
  current: number;
  /** Total docs during "mapping"; 0 otherwise. */
  total: number;
}

export interface AskAiCompletePayload {
  runId: string;
  answerMarkdown: string;
  /** In `[M#]` order — `sources[i]` ↔ `[M{i+1}]`. */
  sources: SourceMeeting[];
}

export interface AskAiErrorPayload {
  runId: string;
  message: string;
  /** True = user cancellation: return to idle silently, never show an error. */
  cancelled: boolean;
}

// ---------------------------------------------------------------------------
// Scope helpers
// ---------------------------------------------------------------------------

export type DatePreset = 'all' | '30d' | '7d' | 'custom';

/**
 * Parse a `<input type="date">` value (`YYYY-MM-DD`) as a LOCAL calendar day.
 * `new Date('YYYY-MM-DD')` would parse as UTC midnight — off by the TZ offset.
 */
function parseLocalDay(day: string): Date | null {
  const m = /^(\d{4})-(\d{2})-(\d{2})$/.exec(day);
  if (!m) return null;
  return new Date(Number(m[1]), Number(m[2]) - 1, Number(m[3]));
}

/**
 * Build the IPC scope from the page's controls. Empty scope = all meetings.
 *
 * Date bounds are timezone-correct UTC instants: `dateFrom` = local midnight
 * at the start of the first day (inclusive); `dateTo` = local midnight at the
 * start of the day AFTER the last selected day (exclusive upper bound). So
 * "7d" = [start of local day (now − 6d), start of tomorrow local) — 7 whole
 * local calendar days including today.
 */
export function buildScope(opts: {
  preset: DatePreset;
  customFrom?: string;
  customTo?: string;
  personId?: string | null;
  now?: Date; // injectable for tests
}): AggregationScope {
  const scope: AggregationScope = {};
  if (opts.preset === '30d' || opts.preset === '7d') {
    const now = opts.now ?? new Date();
    const days = opts.preset === '30d' ? 30 : 7;
    // Date-constructor arithmetic normalizes month/year rollover for us.
    const from = new Date(now.getFullYear(), now.getMonth(), now.getDate() - (days - 1));
    const to = new Date(now.getFullYear(), now.getMonth(), now.getDate() + 1);
    scope.dateFrom = from.toISOString();
    scope.dateTo = to.toISOString();
  } else if (opts.preset === 'custom') {
    if (opts.customFrom) {
      const from = parseLocalDay(opts.customFrom);
      if (from) scope.dateFrom = from.toISOString();
    }
    if (opts.customTo) {
      const to = parseLocalDay(opts.customTo);
      if (to) {
        to.setDate(to.getDate() + 1); // exclusive: start of the day after
        scope.dateTo = to.toISOString();
      }
    }
  }
  if (opts.personId) scope.personId = opts.personId;
  return scope;
}

// ---------------------------------------------------------------------------
// Label helpers
// ---------------------------------------------------------------------------

/**
 * Whether an Enter keypress in the question input starts the run. IME
 * composition-confirm Enter (isComposing / legacy keyCode 229) never submits.
 */
export function enterTriggersRun(opts: {
  key: string;
  isComposing: boolean;
  keyCode?: number;
  canAsk: boolean;
}): boolean {
  if (opts.key !== 'Enter') return false;
  if (opts.isComposing || opts.keyCode === 229) return false;
  return opts.canAsk;
}

/** Human stage line for the progress panel. */
export function stageLabel(progress: Pick<AskAiProgressPayload, 'stage' | 'current' | 'total'>): string {
  switch (progress.stage) {
    case 'gathering':
      return 'Searching meetings…';
    case 'mapping':
      return `Reading meeting ${progress.current} of ${progress.total}…`;
    case 'reducing':
      return 'Writing answer…';
  }
}

// ---------------------------------------------------------------------------
// History + saved questions (specs/0038 WS2.a / WS2.b)
//
// Thin, resilient wrappers over the Rust commands (all DTOs serde camelCase).
// Reads degrade to `[]` and log rather than throwing — a persistence hiccup must
// never break the live single-question flow. Writes/deletes throw a friendly
// string so the caller can surface a toast.
// ---------------------------------------------------------------------------

/** One persisted Ask-AI run (WS2.a), most-recent-first from the backend. */
export interface AskAiHistoryEntry {
  /** `"aah-<uuid>"`. */
  id: string;
  question: string;
  /** Serialized `AggregationScope` — re-run/reload round-trips it verbatim. */
  scopeJson: string;
  answerMarkdown: string;
  /** JSON array of `SourceMeeting` — parse for the read-only reload. */
  sourcesJson: string;
  /** Summary provider/model the run followed; `null` when unknown. */
  provider: string | null;
  model: string | null;
  createdAt: string; // ISO-8601 UTC
}

/** A starred, reusable question (WS2.b) that re-runs against fresh data. */
export interface SavedQuestion {
  /** `"saq-<uuid>"`. */
  id: string;
  label: string;
  question: string;
  /** Serialized `AggregationScope` — re-run parses this and passes it through unchanged. */
  scopeJson: string;
  createdAt: string; // ISO-8601 UTC
  updatedAt: string; // ISO-8601 UTC
  /** ISO-8601 of the last run; `null` until first run. Stamped when the cached answer is written. */
  lastRunAt: string | null;
  /** Cached answer markdown from the last run (specs/0038 #4); `null` until first run. */
  lastAnswerMarkdown: string | null;
  /** Cached JSON array of `SourceMeeting` for the last answer; `null` until first run. */
  lastSourcesJson: string | null;
}

/** Most-recent-first Ask-AI history. Returns `[]` on any failure (never throws). */
export async function listAskAiHistory(limit?: number): Promise<AskAiHistoryEntry[]> {
  try {
    const result = await invoke<AskAiHistoryEntry[]>('api_list_ask_ai_history', { limit });
    return Array.isArray(result) ? result : [];
  } catch (err) {
    console.warn('[ask-ai] listAskAiHistory failed:', err);
    return [];
  }
}

/** Delete one history entry. Throws a friendly message on failure. */
export async function deleteAskAiHistory(id: string): Promise<boolean> {
  try {
    return await invoke<boolean>('api_delete_ask_ai_history', { id });
  } catch (err) {
    console.error('[ask-ai] deleteAskAiHistory failed:', err);
    throw new Error('Could not delete that history entry. Please try again.');
  }
}

/**
 * Parse the `sourcesJson` stored on a history entry back into `SourceMeeting[]`
 * for the read-only reload. Malformed/absent JSON degrades to `[]` (the answer
 * still renders; citation chips just won't resolve).
 */
export function parseSources(sourcesJson: string): SourceMeeting[] {
  try {
    const parsed = JSON.parse(sourcesJson);
    return Array.isArray(parsed) ? (parsed as SourceMeeting[]) : [];
  } catch {
    return [];
  }
}

/**
 * Parse a stored `scopeJson` back into an `AggregationScope` for a saved-question
 * re-run — passed through to `api_ask_ai_run` unchanged. Degrades to `{}` (all
 * meetings) if the stored value is malformed.
 */
export function parseScope(scopeJson: string): AggregationScope {
  try {
    const parsed = JSON.parse(scopeJson);
    return parsed && typeof parsed === 'object' ? (parsed as AggregationScope) : {};
  } catch {
    return {};
  }
}

/**
 * The cached answer to show on a saved question's sub-page (specs/0038 #4), or `null` when it has
 * never been run. Pure so the empty-vs-cached decision is testable without a Tauri shell — the
 * sub-page renders the empty state exactly when this returns `null`, and an answer otherwise.
 */
export function cachedAnswerFor(
  sq: Pick<SavedQuestion, 'lastAnswerMarkdown' | 'lastSourcesJson'>,
): { markdown: string; sources: SourceMeeting[] } | null {
  if (!sq.lastAnswerMarkdown) return null;
  return {
    markdown: sq.lastAnswerMarkdown,
    sources: parseSources(sq.lastSourcesJson ?? '[]'),
  };
}

/** Star the current question + scope. `scopeJson` must be the same scope passed to `api_ask_ai_run`. */
export async function createSavedQuestion(
  label: string,
  question: string,
  scopeJson: string,
): Promise<SavedQuestion> {
  try {
    return await invoke<SavedQuestion>('api_create_saved_question', { label, question, scopeJson });
  } catch (err) {
    console.error('[ask-ai] createSavedQuestion failed:', err);
    throw new Error('Could not save that question. Please try again.');
  }
}

/** All saved questions, most-recently-created first. Returns `[]` on any failure (never throws). */
export async function listSavedQuestions(): Promise<SavedQuestion[]> {
  try {
    const result = await invoke<SavedQuestion[]>('api_list_saved_questions');
    return Array.isArray(result) ? result : [];
  } catch (err) {
    console.warn('[ask-ai] listSavedQuestions failed:', err);
    return [];
  }
}

/** Delete one saved question. Throws a friendly message on failure. */
export async function deleteSavedQuestion(id: string): Promise<boolean> {
  try {
    return await invoke<boolean>('api_delete_saved_question', { id });
  } catch (err) {
    console.error('[ask-ai] deleteSavedQuestion failed:', err);
    throw new Error('Could not delete that saved question. Please try again.');
  }
}

/**
 * Load one saved question by id for its sub-page (specs/0038 #4). The sub-page renders the
 * CACHED answer (`lastAnswerMarkdown`/`lastSourcesJson`) without re-running the LLM. Returns
 * `null` when the id is unknown or on any failure (the page then shows a not-found state).
 */
export async function getSavedQuestion(id: string): Promise<SavedQuestion | null> {
  try {
    const result = await invoke<SavedQuestion | null>('api_get_saved_question', { id });
    return result ?? null;
  } catch (err) {
    console.warn('[ask-ai] getSavedQuestion failed:', err);
    return null;
  }
}

/**
 * Cache the answer from a Rerun on the saved-question sub-page (specs/0038 #4): persists the
 * answer markdown + sources so the next open renders instantly with no LLM call. Throws a
 * friendly message on failure so the caller can surface a toast.
 */
export async function updateSavedQuestionAnswer(
  id: string,
  answerMarkdown: string,
  sourcesJson: string,
): Promise<boolean> {
  try {
    return await invoke<boolean>('api_update_saved_question_answer', {
      id,
      answerMarkdown,
      sourcesJson,
    });
  } catch (err) {
    console.error('[ask-ai] updateSavedQuestionAnswer failed:', err);
    throw new Error('Could not save the answer. Please try again.');
  }
}
