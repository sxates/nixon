/**
 * Typed `invoke` wrappers for the Topics commands (specs/0089, src-tauri/src/topics/
 * commands.rs). One wrapper per command; Tauri camelCases the argument names. Errors are
 * the backend's user-facing strings (rejections), left for callers to toast.
 */

import { invoke } from '@tauri-apps/api/core';
import type {
  EntryBody,
  Topic,
  TopicDetail,
  TopicLink,
  TopicLinkKind,
  TopicStatus,
  TopicSummary,
} from '@/types';

export const listTopics = (status?: TopicStatus) =>
  invoke<TopicSummary[]>('api_list_topics', { status: status ?? null });

export const getTopic = (id: string) => invoke<TopicDetail>('api_get_topic', { id });

export interface CreateTopicInput {
  name: string;
  description?: string | null;
  linkMeetingId?: string | null;
  /** true = link the meeting's whole series; false = just that meeting. */
  linkSeries?: boolean;
  backfill?: boolean;
}

export const createTopic = (input: CreateTopicInput) =>
  invoke<Topic>('api_create_topic', {
    name: input.name,
    description: input.description ?? null,
    linkMeetingId: input.linkMeetingId ?? null,
    linkSeries: input.linkSeries ?? false,
    backfill: input.backfill ?? false,
  });

export interface UpdateTopicInput {
  name?: string;
  description?: string;
  status?: TopicStatus;
}

export const updateTopic = (id: string, patch: UpdateTopicInput) =>
  invoke<Topic>('api_update_topic', {
    id,
    name: patch.name ?? null,
    description: patch.description ?? null,
    status: patch.status ?? null,
  });

export const setTopicNotes = (id: string, notesMarkdown: string, notesJson?: string | null) =>
  invoke<void>('api_set_topic_notes', { id, notesMarkdown, notesJson: notesJson ?? null });

export const deleteTopic = (id: string) => invoke<void>('api_delete_topic', { id });

export const linkTopic = (
  topicId: string,
  meetingId: string,
  scope: 'meeting' | 'series',
  backfill = false,
) => invoke<TopicLink>('api_link_topic', { topicId, meetingId, scope, backfill });

export const unlinkTopic = (topicId: string, kind: TopicLinkKind, ref: string) =>
  invoke<void>('api_unlink_topic', { topicId, kind, ref });

/** Open and closed topics linked to a meeting. */
export const topicsForMeeting = (meetingId: string) =>
  invoke<Topic[]>('api_topics_for_meeting', { meetingId });

/** An LLM draft of the description; never saved by the backend. */
export const draftTopicDescription = (topicId: string) =>
  invoke<string>('api_draft_topic_description', { topicId });

export const editTopicEntry = (entryId: string, body: EntryBody) =>
  invoke<void>('api_edit_topic_entry', { entryId, body });

export const revertTopicEntry = (entryId: string) =>
  invoke<void>('api_revert_topic_entry', { entryId });

/** Rejects with "Edited entries can't be rewritten." when the entry was edited. */
export const rewriteTopicEntry = (entryId: string) =>
  invoke<void>('api_rewrite_topic_entry', { entryId });

/** Returns the new entry id. */
export const addManualTopicEntry = (topicId: string, body: EntryBody) =>
  invoke<string>('api_add_manual_topic_entry', { topicId, body });

export const deleteTopicEntry = (entryId: string) =>
  invoke<void>('api_delete_topic_entry', { entryId });

export const resolveTopicSuggestion = (id: string, accept: boolean) =>
  invoke<void>('api_resolve_topic_suggestion', { id, accept });

/** Tag a to-do to an open topic; `null` untags. */
export const setActionItemTopic = (id: string, topicId: string | null) =>
  invoke<void>('api_set_action_item_topic', { id, topicId });

/** RFC 3339 time -> "Nov 3" (local day); "" when unparseable. */
export function formatDecisionDate(iso: string): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return '';
  return d.toLocaleDateString('en-US', { month: 'short', day: 'numeric' });
}
