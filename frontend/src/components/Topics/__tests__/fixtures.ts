// Invented Topic fixtures for the topic page tests (specs/0089).
import type { Topic, TopicDetail, TopicEntry, TopicSuggestion } from '@/types';

export const topic = (over: Partial<Topic> = {}): Topic => ({
  id: 'topic-1',
  name: 'Atlas CRM rebuild',
  description: 'Replacing the legacy CRM.',
  status: 'open',
  notesMarkdown: null,
  notesJson: null,
  createdAt: '2026-10-01T12:00:00Z',
  updatedAt: '2026-10-01T12:00:00Z',
  closedAt: null,
  ...over,
});

export const entry = (over: Partial<TopicEntry> = {}): TopicEntry => {
  const body = {
    bullets: [{ text: 'Field mapping is done.', t: 75 }],
    decisions: [{ id: 'dec-1', text: 'Migrate in two waves.', t: 312 }],
  };
  return {
    id: 'te-1',
    topicId: 'topic-1',
    meetingId: 'm-1',
    meetingTitle: 'Product sync',
    kind: 'series',
    occurredAt: '2026-11-03T12:00:00Z',
    aiBody: body,
    body,
    editedAt: null,
    sourceDeletedAt: null,
    createdAt: '2026-11-03T12:00:00Z',
    revisions: [],
    ...over,
  };
};

export const noteSuggestion = (over: Partial<TopicSuggestion> = {}): TopicSuggestion => ({
  id: 'ts-1',
  topicId: 'topic-1',
  kind: 'note',
  meetingId: 'm-2',
  meetingTitle: 'Support weekly',
  tStart: 220,
  targetEntryId: null,
  targetDecisionId: null,
  targetDecisionText: null,
  targetOccurredAt: null,
  text: 'Support wants a read-only view during the pilot.',
  status: 'pending',
  createdAt: '2026-11-10T12:00:00Z',
  ...over,
});

export const revisionSuggestion = (over: Partial<TopicSuggestion> = {}): TopicSuggestion =>
  noteSuggestion({
    id: 'ts-2',
    kind: 'revision',
    targetEntryId: 'te-1',
    targetDecisionId: 'dec-1',
    targetDecisionText: 'Migrate in two waves.',
    targetOccurredAt: '2026-11-03T12:00:00Z',
    text: 'Start with the largest region instead.',
    ...over,
  });

export const detail = (over: Partial<TopicDetail> = {}): TopicDetail => ({
  topic: topic(),
  links: [],
  entries: [entry()],
  suggestions: [],
  todos: [],
  ...over,
});
