// Topics (specs/0089) handlers for the screenshot mock. Loaded BEFORE tauri-mock.js (run.mjs
// and screenshot.mjs prepend this file); tauri-mock.js spreads
// `__nixonTopicsMock({ MEETINGS, todoStore })` into FIXTURES. Shapes mirror
// src-tauri/src/topics/{model,links,commands}.rs (camelCase). All content is invented.
globalThis.__nixonTopicsMock = function ({ MEETINGS, todoStore }) {
  const daysAgo = (n, h = 14) => {
    const d = new Date();
    d.setDate(d.getDate() - n);
    d.setHours(h, 0, 0, 0);
    return d.toISOString();
  };
  const titleOf = (id) => (MEETINGS.find((m) => m.id === id) || {}).title || null;
  const ATLAS = 'topic-atlas';
  const CLOSED = 'topic-vendor-eval';

  let STORE = null;
  function store() {
    if (STORE) return STORE;
    const iso = daysAgo(0);
    const aiBody = (bullets, decisions = []) => ({ bullets, decisions });
    const entry = (e) => ({
      topicId: ATLAS, meetingTitle: e.meetingId ? titleOf(e.meetingId) : null, kind: 'series',
      aiBody: e.body, editedAt: null, sourceDeletedAt: null, createdAt: e.occurredAt, revisions: [], ...e,
    });
    const oldDecision = { id: 'dec-1', text: 'Migrate accounts in two waves, starting with the smallest region.', t: 312 };
    STORE = {
      topics: [
        {
          id: ATLAS, name: 'Atlas CRM rebuild', status: 'open',
          description: 'Replacing the legacy CRM with Atlas: data migration, rollout order, and the cutover date.',
          notesMarkdown: 'Check the export limits with the vendor before wave two.', notesJson: null,
          createdAt: daysAgo(20), updatedAt: daysAgo(1), closedAt: null,
        },
        {
          id: CLOSED, name: 'Vendor evaluation', status: 'closed',
          description: 'Shortlist and pick the enclosure supplier.',
          notesMarkdown: null, notesJson: null, createdAt: daysAgo(60), updatedAt: daysAgo(30), closedAt: daysAgo(30),
        },
      ],
      links: [
        { topicId: ATLAS, kind: 'series', ref: 'series-product-sync', title: 'product sync', label: titleOf('demo-01') || 'Product sync' },
        { topicId: CLOSED, kind: 'meeting', ref: 'demo-05', title: null, label: titleOf('demo-05') || 'Vendor call' },
      ],
      entries: [
        // Newest first, as the backend returns them.
        entry({
          id: 'te-3', meetingId: 'demo-01', occurredAt: daysAgo(0, 14),
          body: aiBody([], []), aiBody: aiBody([], []),
        }),
        entry({
          id: 'te-2', meetingId: 'demo-04', kind: 'mention', occurredAt: daysAgo(5, 9),
          aiBody: aiBody([{ text: 'Export limits may slow the second wave.', t: 140 }]),
          body: aiBody([{ text: 'Export limits may slow the second wave; ask the vendor for a raised cap.', t: 140 }]),
          editedAt: daysAgo(4, 10),
        }),
        entry({
          id: 'te-1', meetingId: 'demo-02', occurredAt: daysAgo(12, 11),
          body: aiBody([{ text: 'Field mapping is done for accounts and contacts.', t: 75 }], [oldDecision]),
          aiBody: aiBody([{ text: 'Field mapping is done for accounts and contacts.', t: 75 }], [oldDecision]),
          revisions: [{
            id: 'tr-1', entryId: 'te-1', decisionId: 'dec-1', sourceMeetingId: 'demo-01', tStart: 410,
            text: 'Wave two now waits until the export cap is raised.', sourceDeletedAt: null, createdAt: daysAgo(3),
          }],
        }),
      ],
      suggestions: [
        {
          id: 'ts-1', topicId: ATLAS, kind: 'note', meetingId: 'demo-04', meetingTitle: titleOf('demo-04'), tStart: 220,
          targetEntryId: null, targetDecisionId: null, targetDecisionText: null, targetOccurredAt: null,
          text: 'Support wants a read-only view of Atlas during the pilot.', status: 'pending', createdAt: daysAgo(1),
        },
        {
          id: 'ts-2', topicId: ATLAS, kind: 'revision', meetingId: 'demo-01', meetingTitle: titleOf('demo-01'), tStart: 640,
          targetEntryId: 'te-1', targetDecisionId: 'dec-1', targetDecisionText: oldDecision.text, targetOccurredAt: daysAgo(12, 11),
          text: 'Start with the largest region instead, to surface scale issues early.', status: 'pending', createdAt: daysAgo(0),
        },
      ],
    };
    return STORE;
  }

  const topicOf = (id) => store().topics.find((t) => t.id === id);
  const todosOf = (id) => todoStore().filter((r) => r.topicId === id);
  const summary = (t) => {
    const s = store();
    const es = s.entries.filter((e) => e.topicId === t.id);
    return {
      id: t.id, name: t.name, description: t.description, status: t.status,
      lastActivity: es.length ? es[0].occurredAt : null,
      pendingCount: s.suggestions.filter((x) => x.topicId === t.id && x.status === 'pending').length,
      meetingCount: new Set(es.map((e) => e.meetingId).filter(Boolean)).size,
    };
  };
  // One to-do carries a topic so the To-do shot shows the chip.
  const tagOne = () => {
    const r = todoStore().find((x) => x.meetingId === 'demo-01' && x.status === 'open');
    if (r && r.topicId === undefined) { r.topicId = ATLAS; r.topicName = 'Atlas CRM rebuild'; }
  };

  return {
    api_list_topics: ({ status }) => { tagOne(); return store().topics.filter((t) => !status || t.status === status).map(summary); },
    api_get_topic: ({ id }) => {
      tagOne();
      const t = topicOf(id);
      if (!t) return Promise.reject('Topic not found.');
      const s = store();
      return {
        topic: t, links: s.links.filter((l) => l.topicId === id), entries: s.entries.filter((e) => e.topicId === id),
        suggestions: s.suggestions.filter((x) => x.topicId === id && x.status === 'pending'), todos: todosOf(id),
      };
    },
    api_create_topic: ({ name, description }) => {
      const t = { id: 'topic-' + Date.now(), name, description: description || '', status: 'open', notesMarkdown: null, notesJson: null, createdAt: daysAgo(0), updatedAt: daysAgo(0), closedAt: null };
      store().topics.unshift(t);
      return t;
    },
    api_update_topic: ({ id, name, description, status }) => {
      const t = topicOf(id);
      if (name != null) t.name = name;
      if (description != null) t.description = description;
      if (status != null) { t.status = status; t.closedAt = status === 'closed' ? daysAgo(0) : null; }
      return t;
    },
    api_set_topic_notes: ({ id, notesMarkdown, notesJson }) => { const t = topicOf(id); t.notesMarkdown = notesMarkdown; t.notesJson = notesJson; },
    api_delete_topic: ({ id }) => { const s = store(); s.topics = s.topics.filter((t) => t.id !== id); },
    api_link_topic: ({ topicId, meetingId, scope }) => ({ topicId, kind: scope, ref: meetingId, title: null, label: titleOf(meetingId) || meetingId }),
    api_unlink_topic: ({ topicId, kind, ref }) => { const s = store(); s.links = s.links.filter((l) => !(l.topicId === topicId && l.kind === kind && l.ref === ref)); },
    api_topics_for_meeting: ({ meetingId }) => {
      const s = store();
      const ids = new Set(s.entries.filter((e) => e.meetingId === meetingId).map((e) => e.topicId));
      if (meetingId === 'demo-01') ids.add(ATLAS);
      return s.topics.filter((t) => ids.has(t.id));
    },
    api_draft_topic_description: () => 'Replacing the legacy CRM with Atlas, covering data migration, rollout order, and the cutover date.',
    api_edit_topic_entry: ({ entryId, body }) => { const e = store().entries.find((x) => x.id === entryId); e.body = body; e.editedAt = daysAgo(0); },
    api_revert_topic_entry: ({ entryId }) => { const e = store().entries.find((x) => x.id === entryId); if (e.aiBody) e.body = e.aiBody; e.editedAt = null; },
    api_rewrite_topic_entry: () => undefined,
    api_add_manual_topic_entry: ({ topicId, body }) => {
      const id = 'te-' + Date.now();
      store().entries.unshift({ id, topicId, meetingId: null, meetingTitle: null, kind: 'manual', occurredAt: daysAgo(0), aiBody: null, body, editedAt: null, sourceDeletedAt: null, createdAt: daysAgo(0), revisions: [] });
      return id;
    },
    api_delete_topic_entry: ({ entryId }) => { const s = store(); s.entries = s.entries.filter((e) => e.id !== entryId); },
    api_resolve_topic_suggestion: ({ id, accept }) => { const x = store().suggestions.find((s) => s.id === id); if (x) x.status = accept ? 'accepted' : 'dismissed'; },
    api_set_action_item_topic: ({ id, topicId }) => {
      const r = todoStore().find((x) => x.id === id);
      const t = topicId && topicOf(topicId);
      if (r) { r.topicId = topicId || null; r.topicName = t ? t.name : null; }
    },
  };
};
