'use client';

/**
 * ItemDetail (specs/0088 §UI "Detail page") — one to-do in full: title, due date, the
 * state-appropriate triage / status actions, autosaving Notes, and the meetings that
 * mention it (Sources).
 *
 * Notes autosave 600 ms after typing stops and on blur, and flush when the page unmounts,
 * so a navigation mid-debounce never drops text. The title uses `EditableText` until the
 * mention editor (`@person`); on save a trailing `(Nov 20)` sets the due date like quick-add.
 * A refresh that started before a notes save never overwrites the saved text, and a failed
 * refresh keeps an already-loaded page (toast) instead of swapping in the error callout.
 */

import { useCallback, useEffect, useMemo, useRef, useState, type Dispatch, type SetStateAction } from 'react';
import { useRouter } from 'next/navigation';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { toast } from 'sonner';
import { Button } from '@/components/ui/button';
import { Callout } from '@/components/ui/callout';
import { Checkbox } from '@/components/ui/checkbox';
import { EmptyState, PageLoading } from '@/components/ui/empty-state';
import { Panel } from '@/components/ui/panel';
import { avatarColorClass } from '@/lib/avatar-colors';
import { ListRow } from '@/components/ui/list-row';
import { PageHeader } from '@/components/ui/page-header';
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select';
import { Textarea } from '@/components/ui/textarea';
import { useTopics } from '@/hooks/useTopics';
import { setActionItemTopic } from '@/lib/topics';
import { useActionItemMutations } from '@/hooks/useActionItemMutations';
import { formatMeetingDate } from '@/lib/format-date';
import { parseTodoText, titleSegments } from '@/lib/todo-parse';
import { PAGE_CONTENT_WIDTH, PAGE_GUTTER } from '@/lib/page-width';
import { cn } from '@/lib/utils';
import type { ActionItem, ActionItemDetail, Person } from '@/types';
import { DueDateControl } from './DueDateControl';
import { MentionInput, type MentionTopic } from './MentionInput';

export const NOTES_DEBOUNCE_MS = 600;

type SaveState = 'idle' | 'saving' | 'saved';

const errorText = (err: unknown) => (err instanceof Error ? err.message : String(err));

export function ItemDetail({ id }: { id: string }) {
  const router = useRouter();
  const [item, setItem] = useState<ActionItem | null>(null);
  const [sources, setSources] = useState<ActionItemDetail['sources']>([]);
  const [status, setStatus] = useState<'loading' | 'ready' | 'missing' | 'error'>('loading');
  const [people, setPeople] = useState<Person[]>([]);

  // ---- load + refresh ----
  const notesDraft = useRef('');
  const notesSaved = useRef('');
  const notesDirty = useRef(false);
  const [notes, setNotesText] = useState('');

  const loadSeq = useRef(0); // latest load wins
  const notesEpoch = useRef(0); // bumped by every notes edit/save
  const loadedOnce = useRef(false);

  const load = useCallback(async () => {
    const seq = ++loadSeq.current;
    const epoch = notesEpoch.current;
    try {
      const d = await invoke<ActionItemDetail>('api_get_action_item_detail', { id });
      if (seq !== loadSeq.current) return;
      // The topic travels beside `item` on the wire; fold it in so the field can read it.
      setItem({ ...d.item, topicId: d.topicId ?? null, topicName: d.topicName ?? null });
      setSources(d.sources ?? []);
      // Never overwrite text the user is mid-way through typing, or text saved while this
      // request was in flight (its notes are older than what we hold).
      if (!notesDirty.current && epoch === notesEpoch.current) {
        const text = d.item.notesMarkdown ?? '';
        notesDraft.current = text;
        notesSaved.current = text;
        setNotesText(text);
      }
      loadedOnce.current = true;
      setStatus('ready');
    } catch (err) {
      if (seq !== loadSeq.current) return;
      console.error('Failed to load to-do:', err);
      if (/no longer exists/i.test(errorText(err))) {
        setStatus('missing');
      } else if (loadedOnce.current) {
        toast.error('Could not refresh this to-do', { description: errorText(err) });
      } else {
        setStatus('error');
      }
    }
  }, [id]);

  useEffect(() => {
    void load();
  }, [load]);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    void listen('action-items-updated', () => void load())
      .then((fn) => (cancelled ? fn() : (unlisten = fn)))
      .catch(() => {});
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [load]);

  useEffect(() => {
    void (async () => {
      try {
        const result = await invoke<Person[]>('api_list_people');
        setPeople(Array.isArray(result) ? result : []);
      } catch (err) {
        console.error('Failed to load people:', err);
      }
    })();
  }, []);
  const nameById = useMemo(() => new Map(people.map((p) => [p.id, p.displayName])), [people]);

  // ---- mutations: the shared hook drives a one-item list ----
  const setItems = useCallback<Dispatch<SetStateAction<ActionItem[]>>>((update) => {
    setItem((prev) => {
      const list = prev ? [prev] : [];
      const next = typeof update === 'function' ? update(list) : update;
      return next[0] ?? null;
    });
  }, []);
  const { setStatus: setItemStatus, editDescription, setDueDate, triage, setNotes } =
    useActionItemMutations<ActionItem>({ setItems, meetingId: null });

  const runTriage = async (action: Parameters<typeof triage>[1]) => {
    try {
      await triage(id, action);
      await load();
    } catch (err) {
      console.error('Failed to update to-do:', err);
      toast.error('Could not update the to-do', { description: errorText(err) });
    }
  };

  // ---- title ----
  const [titleReset, setTitleReset] = useState(0);
  const { data: openTopics } = useTopics('open');
  const topicOptions = useMemo<MentionTopic[]>(
    () => (openTopics ?? []).map((t) => ({ id: t.id, name: t.name })),
    [openTopics],
  );
  // The topic is tagged on its own command (specs/0089); `load` re-reads the name after.
  const changeTopic = async (topicId: string | null) => {
    try {
      await setActionItemTopic(id, topicId);
      await load();
    } catch (err) {
      console.error('Failed to set the topic:', err);
      toast.error('Could not change the topic', { description: errorText(err) });
    }
  };
  const [pickedTopic, setPickedTopic] = useState<MentionTopic | null>(null);
  const finishTitle = async (serialized: string) => {
    if (!item) return;
    const topic = pickedTopic;
    setPickedTopic(null);
    try {
      const { title, dueDate } = parseTodoText(serialized.trim(), new Date());
      if (!title) return; // empty title: revert (below)
      if (title !== item.description) await editDescription(item, title);
      if (dueDate) await setDueDate(item, dueDate);
      if (topic && topic.id !== item.topicId) await changeTopic(topic.id);
    } finally {
      // Always show the saved text: a parsed `(Nov 20)` leaves the field, an empty title reverts.
      setTitleReset((n) => n + 1);
    }
  };

  // ---- notes autosave ----
  const [saveState, setSaveState] = useState<SaveState>('idle');
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const mounted = useRef(true);

  const flushNotes = useCallback(async () => {
    if (timer.current) {
      clearTimeout(timer.current);
      timer.current = null;
    }
    const text = notesDraft.current;
    notesEpoch.current += 1;
    if (text === notesSaved.current) {
      notesDirty.current = false;
      return;
    }
    if (mounted.current) setSaveState('saving');
    try {
      await setNotes(id, text.trim() ? text : null);
      notesSaved.current = text;
      notesEpoch.current += 1;
      if (notesDraft.current === text) notesDirty.current = false;
      if (mounted.current) setSaveState('saved');
    } catch (err) {
      console.error('Failed to save notes:', err);
      toast.error('Could not save your notes', { description: errorText(err) });
      if (mounted.current) setSaveState('idle');
    }
  }, [id, setNotes]);

  const flushRef = useRef(flushNotes);
  flushRef.current = flushNotes;
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
      void flushRef.current();
    };
  }, []);

  const onNotesChange = (value: string) => {
    notesDraft.current = value;
    notesDirty.current = true;
    notesEpoch.current += 1;
    setNotesText(value);
    setSaveState('idle');
    if (timer.current) clearTimeout(timer.current);
    timer.current = setTimeout(() => void flushRef.current(), NOTES_DEBOUNCE_MS);
  };

  // ---- render ----
  if (status === 'loading') return <PageLoading className="bg-background" />;
  if (status === 'missing' || (status === 'ready' && !item)) {
    return (
      <div className="flex h-page flex-col bg-background">
        <PageHeader
          width="page"
          title="To-do"
          back={{ label: 'To-do', onClick: () => router.push('/tasks') }}
        />
        <EmptyState
          title="This to-do no longer exists"
          action={
            <Button variant="outline" size="sm" onClick={() => router.push('/tasks')}>
              Back to To-do
            </Button>
          }
        />
      </div>
    );
  }
  if (status === 'error' || !item) {
    return (
      <div className="flex h-page flex-col bg-background">
        <PageHeader width="page" title="To-do" back={{ label: 'To-do', onClick: () => router.push('/tasks') }} />
        <div className={cn('mx-auto w-full', PAGE_CONTENT_WIDTH, PAGE_GUTTER)}>
          <Callout tone="error" title="Could not load this to-do">
            Try again in a moment.
          </Callout>
        </div>
      </div>
    );
  }

  const completed = item.status === 'completed';
  const dismissed = item.status === 'dismissed';
  const displayTitle = titleSegments(item.description, nameById)
    .map((s) => (s.kind === 'person' ? `@${s.name}` : s.text))
    .join('');

  let actions: React.ReactNode;
  if (dismissed) {
    actions = (
      <Button size="sm" variant="outline" onClick={() => void runTriage('restore')}>
        Restore
      </Button>
    );
  } else if (item.triage === 'suggested') {
    actions = (
      <>
        <Button size="sm" variant="brand" onClick={() => void runTriage('add_to_list')}>
          Add to my list
        </Button>
        <Button size="sm" variant="outline" onClick={() => void runTriage('dismiss')}>
          Dismiss
        </Button>
      </>
    );
  } else if (item.triage === 'record') {
    actions = (
      <Button size="sm" variant="outline" onClick={() => void runTriage('adopt')}>
        Adopt
      </Button>
    );
  }
  // A listed item is yours to tick off: the checkbox sits beside the title, as on the list.
  const listed = !dismissed && item.triage === 'listed';

  return (
    <div className="flex h-page animate-fade-in flex-col bg-background">
      <PageHeader
        width="page"
        title="To-do"
        back={{ label: 'To-do', onClick: () => router.push('/tasks') }}
      />
      {/* pt-1: the scroll box clips overflow, and the title's focus ring sits outside it. */}
      <div className={cn('flex-1 overflow-y-auto pb-12 pt-1', PAGE_GUTTER)}>
        <div className={cn('mx-auto flex w-full flex-col gap-6', PAGE_CONTENT_WIDTH)}>
          <div className="flex flex-col gap-3">
            <div className="flex items-start gap-3">
              {listed && (
                <Checkbox
                  checked={completed}
                  aria-label={
                    completed ? `Mark "${displayTitle}" as open` : `Mark "${displayTitle}" as completed`
                  }
                  onCheckedChange={(next) => void setItemStatus(item, next ? 'completed' : 'open')}
                  className="mt-2.5 shrink-0"
                />
              )}
              <div className="min-w-0 flex-1">
                <MentionInput
                  value={item.description}
                  onSubmit={(v) => void finishTitle(v)}
                  onCancel={() => {
                    setPickedTopic(null);
                    setTitleReset((n) => n + 1);
                  }}
                  resetKey={titleReset}
                  submitOnBlur
                  multiline
                  ariaLabel="To-do title"
                  placeholder="Title"
                  variant="bare"
                  size="title"
                  topics={topicOptions}
                  topicId={pickedTopic?.id ?? item.topicId ?? null}
                  onTopicChange={setPickedTopic}
                  className={cn('-mx-1', completed && 'text-muted-foreground line-through')}
                />
              </div>
            </div>
            <div className={cn('flex flex-wrap items-center gap-3', listed && 'pl-7')}>
              {/* -ml-1 cancels the control's hover padding so its text lines up with the title's. */}
              <DueDateControl item={item} onSetDueDate={setDueDate} size="body" className="-ml-1" />
              {actions}
            </div>
            <div className={cn('flex items-center gap-2', listed && 'pl-7')}>
              <span className="u-section-label-sm">Topic</span>
              <Select
                value={item.topicId ?? 'none'}
                onValueChange={(v) => void changeTopic(v === 'none' ? null : v)}
              >
                <SelectTrigger aria-label="Topic" className="h-8 w-56 text-sm">
                  <SelectValue placeholder="None" />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="none">None</SelectItem>
                  {/* A tagged topic that has since closed is not in the open list. */}
                  {item.topicId && item.topicName && !topicOptions.some((t) => t.id === item.topicId) && (
                    <SelectItem value={item.topicId} disabled>
                      {item.topicName}
                    </SelectItem>
                  )}
                  {topicOptions.map((t) => (
                    <SelectItem key={t.id} value={t.id}>
                      {t.name}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
          </div>

          <section className="flex flex-col gap-2">
            <div className="flex items-baseline justify-between">
              <h2 className="u-section-label">
                Notes
              </h2>
              <span className="min-w-16 text-right text-caption text-muted-foreground" aria-live="polite">
                {saveState === 'saving' ? 'Saving…' : saveState === 'saved' ? 'Saved' : ''}
              </span>
            </div>
            <Textarea
              aria-label="Notes"
              value={notes}
              onChange={(e) => onNotesChange(e.target.value)}
              onBlur={() => void flushNotes()}
              placeholder="Add notes"
              rows={6}
            />
          </section>

          <section aria-labelledby="todo-sources" className="flex flex-col gap-2">
            <h2 id="todo-sources" className="u-section-label">
              Sources
            </h2>
            {sources.length === 0 ? (
              <p className="text-sm text-muted-foreground">Added by hand, not from a meeting.</p>
            ) : (
              // Styled as the Meetings list: a panel of ruled rows, each with the meeting's spine.
              <Panel className="overflow-hidden p-0 [&>*:last-child]:border-b-0">
                {sources.map((s) => (
                  <ListRow
                    key={s.meetingId}
                    ruled
                    lead={
                      <span
                        className={cn('block h-4 w-[3px] flex-none', avatarColorClass(s.meetingId))}
                        aria-hidden="true"
                      />
                    }
                    title={s.meetingTitle ?? 'Untitled meeting'}
                    meta={s.context ?? undefined}
                    trailing={
                      <span className="u-row-meta tabular-nums">
                        {formatMeetingDate(s.meetingCreatedAt)}
                      </span>
                    }
                    hint="Open this meeting"
                    onClick={() => router.push(`/meeting-details?id=${s.meetingId}`)}
                  />
                ))}
              </Panel>
            )}
          </section>
        </div>
      </div>
    </div>
  );
}
