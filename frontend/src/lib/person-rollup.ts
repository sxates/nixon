/**
 * Person roll-up (specs/0038 WS5.b) — IPC types + thin `invoke` wrappers for the
 * People directory's person detail panel.
 *
 * Two backend commands, mirroring the Rust `aggregation::rollup` serde contracts
 * (`rename_all = "camelCase"`):
 *   - `api_recent_meetings_with_person` — a fast, no-LLM list of the person's recent
 *     meetings (load immediately when the panel opens).
 *   - `api_person_rollup` — an awaited, slow LLM synthesis ("recent themes / open
 *     threads / last few meetings with {person}" with `[M#]` citations). No progress
 *     events; the caller shows a spinner while it runs.
 *
 * The roll-up return reuses the Ask-AI answer shape (`{ markdown, sources }`), so its
 * `sources` are the same `SourceMeeting[]` the Ask-AI / prep answers render.
 */

import { invoke } from '@tauri-apps/api/core';
import type { SourceMeeting } from '@/lib/ask-ai';

/** One recent meeting a person was part of — the plain (no-LLM) list. */
export interface RecentPersonMeeting {
  id: string;
  title: string;
  /** ISO-8601 UTC — the meeting's effective start / displayed date. */
  startedAt: string;
}

/**
 * The synthesized "Recent with {person}" brief. Same shape as an Ask-AI answer:
 * `sources[i]` ↔ citation marker `[M{i+1}]`.
 */
export interface PersonRollup {
  markdown: string;
  sources: SourceMeeting[];
}

/** Fast metadata read of the person's recent meetings (no LLM). */
export function fetchRecentMeetingsWithPerson(personId: string): Promise<RecentPersonMeeting[]> {
  return invoke<RecentPersonMeeting[]>('api_recent_meetings_with_person', { personId });
}

/** Awaited LLM synthesis of the person roll-up. Slow — show a spinner while it runs. */
export function fetchPersonRollup(personId: string): Promise<PersonRollup> {
  return invoke<PersonRollup>('api_person_rollup', { personId });
}
