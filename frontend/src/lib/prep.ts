/**
 * Pre-call prep (specs/0036) — IPC types + helpers shared by the Today view and the Prep tab.
 *
 * Mirrors the Rust `aggregation::prep_commands` serde contracts exactly (camelCase):
 *   - commands: api_get_prep, api_ensure_scheduled_meeting, api_regenerate_prep_brief,
 *               api_save_prep_notes, api_get_prep_notes
 *   - events:   prep-brief-progress, prep-brief-complete, prep-brief-error, prep-briefs-updated
 */

import { invoke } from '@tauri-apps/api/core';
import type { ActionItem } from '@/types';
import type { SourceMeeting } from '@/lib/ask-ai';

export type { SourceMeeting };

/** Brief generation state. `absent` = never generated (api_get_prep just kicked one off). */
export type BriefStatus = 'pending' | 'ready' | 'failed' | 'none' | 'absent';

/** One meeting manually linked into this meeting's series (specs/0041 WS4). */
export interface SeriesLinkedMeeting {
  id: string;
  title: string;
  /** RFC3339 — the meeting's effective start / displayed date. */
  startedAt: string;
}

/** `api_get_prep` result — the Prep tab's view model. */
export interface PrepView {
  meetingId: string;
  /** 'scheduled' (upcoming, prep-only) | 'recorded' (has/will have a transcript) | 'notes_only'. */
  origin: string;
  title: string;
  briefStatus: BriefStatus;
  briefMarkdown: string | null;
  briefSources: SourceMeeting[];
  /** Open items carried from the series' prior occurrences, mine-first. Split on `assigneeIsSelf`. */
  openItems: ActionItem[];
  prepNotesMarkdown: string | null;
  prepNotesJson: string | null;
  /** Meetings manually linked into this series (excluding this one), newest first. */
  linkedMeetings: SeriesLinkedMeeting[];
}

export interface PrepNotes {
  prepMarkdown: string | null;
  prepJson: string | null;
}

export type PrepStage = 'gathering' | 'mapping' | 'reducing';

export interface PrepBriefProgressPayload {
  meetingId: string;
  stage: PrepStage;
  current: number;
  total: number;
}

export interface PrepBriefCompletePayload {
  meetingId: string;
  status: BriefStatus;
  markdown: string | null;
  sources: SourceMeeting[];
}

export interface PrepBriefErrorPayload {
  meetingId: string;
  message: string;
}

/** Whether a brief is still being produced (show a spinner). */
export function isBriefLoading(status: BriefStatus): boolean {
  return status === 'pending' || status === 'absent';
}

/** Human line for the brief-generation stage (mirrors Ask-AI's stageLabel). */
export function prepStageLabel(stage: PrepStage): string {
  switch (stage) {
    case 'gathering':
      return 'Gathering past meetings…';
    case 'mapping':
      return 'Reading each meeting…';
    case 'reducing':
      return 'Writing your brief…';
  }
}

/** Split carried-over open items into mine vs. owed-by-others (for the two Prep sub-lists). */
export function splitOpenItems(items: ActionItem[]): { mine: ActionItem[]; others: ActionItem[] } {
  const mine: ActionItem[] = [];
  const others: ActionItem[] = [];
  for (const it of items) {
    (it.assigneeIsSelf ? mine : others).push(it);
  }
  return { mine, others };
}

/** The fields `api_ensure_scheduled_meeting` needs, as an upcoming calendar event carries them. */
export interface ScheduledMeetingSeed {
  /** The calendar occurrence's id. */
  id: string;
  title: string;
  /** ISO-8601 occurrence start. */
  startsAt: string;
  /** The event's `external_id` — series-level, so a recording groups into its series. */
  externalId?: string | null;
}

/**
 * Mint (or return) the `scheduled` meeting row for a calendar occurrence and give back the
 * route to its Prep tab.
 *
 * The Today view has done this inline since specs/0036; it moved here when the five-minute
 * meeting alert grew a **Prep** button (specs/0068) and needed the same two steps from a
 * background component with no router of its own.
 */
export async function prepRouteForEvent(event: ScheduledMeetingSeed): Promise<string> {
  const meetingId = await invoke<string>('api_ensure_scheduled_meeting', {
    calendarEventId: event.id,
    seriesKey: event.externalId ?? null,
    title: event.title,
    occurrenceStart: event.startsAt,
  });
  return `/meeting-details?id=${meetingId}&tab=prep`;
}
