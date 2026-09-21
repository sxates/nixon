'use client';

/**
 * Ask AI (specs/0035) — free-text question answered across meetings.
 *
 * Flow: type a question (optionally scoped by date range / person) → Enter or
 * "Ask" starts the run immediately → `api_ask_ai_run` streams stage progress
 * over the `ask-ai-*` events and lands a cited markdown answer whose `[M#]`
 * markers link to their source meetings. There is no pre-send preview or
 * consent gate (removed 2026-07-04 by product decision): choosing a cloud
 * summary provider in Settings IS the egress consent, and the post-run
 * Sources list shows exactly which meetings were used. Zero matches surface
 * as the run's own readable error — no LLM call happens in that case.
 *
 * Event discipline: listeners are registered for the component's lifetime and
 * every payload is filtered by the active `runId` (stale events from an
 * earlier run are dropped). Because `api_ask_ai_run` resolves the id
 * asynchronously, events that race ahead of the resolve are buffered and
 * replayed once the id is known. Unmount cancels any in-flight run.
 *
 * Cancellation is silent by design: `ask-ai-error` with `cancelled: true`
 * returns to idle without an error state (spec: no stray answer or error
 * after cancelling).
 */

import { Suspense, useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { motion } from 'framer-motion';
import { useRouter, useSearchParams } from 'next/navigation';
import { invoke } from '@tauri-apps/api/core';
import { toast } from 'sonner';
import { ChevronDown, ChevronRight, Clock, Loader2, Sparkles, Star, Trash2 } from 'lucide-react';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu';
import { AnswerMarkdown } from '@/components/AskAI/AnswerMarkdown';
import { SourcesList } from '@/components/AskAI/SourcesList';
import {
  buildScope,
  createSavedQuestion,
  deleteAskAiHistory,
  deleteSavedQuestion,
  enterTriggersRun,
  listAskAiHistory,
  listSavedQuestions,
  parseSources,
  stageLabel,
  type AggregationScope,
  type AskAiCompletePayload,
  type AskAiErrorPayload,
  type AskAiHistoryEntry,
  type AskAiProgressPayload,
  type DatePreset,
  type SavedQuestion,
  type SourceMeeting,
} from '@/lib/ask-ai';
import { formatMeetingDate } from '@/lib/format-date';
import { safeListen } from '@/lib/safe-listen';
import { cn } from '@/lib/utils';
import type { Person } from '@/types';
import { PageHeader } from '@/components/ui/page-header';

const DATE_PRESETS: Array<{ value: DatePreset; label: string }> = [
  { value: 'all', label: 'All time' },
  { value: '30d', label: '30d' },
  { value: '7d', label: '7d' },
  { value: 'custom', label: 'Custom' },
];

type Phase = 'idle' | 'running' | 'done' | 'error';

interface Answer {
  markdown: string;
  sources: SourceMeeting[];
}

type BufferedEvent =
  | { kind: 'progress'; payload: AskAiProgressPayload }
  | { kind: 'complete'; payload: AskAiCompletePayload }
  | { kind: 'error'; payload: AskAiErrorPayload };

function AskPageContent() {
  const router = useRouter();
  const searchParams = useSearchParams();

  // ?q= pre-fills the question (⌘K hands its typed query over).
  const [question, setQuestion] = useState(() => searchParams.get('q') ?? '');
  const [preset, setPreset] = useState<DatePreset>('all');
  const [customFrom, setCustomFrom] = useState('');
  const [customTo, setCustomTo] = useState('');
  const [personId, setPersonId] = useState<string | null>(null);
  const [people, setPeople] = useState<Person[]>([]);

  const [phase, setPhase] = useState<Phase>('idle');
  const [progress, setProgress] = useState<AskAiProgressPayload | null>(null);
  const [answer, setAnswer] = useState<Answer | null>(null);
  const [runError, setRunError] = useState<string | null>(null);
  const [cancelling, setCancelling] = useState(false);

  // WS2 persistence side-panels — additive to the live single-question flow.
  const [history, setHistory] = useState<AskAiHistoryEntry[]>([]);
  const [historyOpen, setHistoryOpen] = useState(false);
  const [savedQuestions, setSavedQuestions] = useState<SavedQuestion[]>([]);
  const [saving, setSaving] = useState(false);
  // Id of the history entry currently shown read-only (so its row can highlight).
  const [viewedHistoryId, setViewedHistoryId] = useState<string | null>(null);

  // Active run id, or null. Events for any other id are stale and ignored.
  const runIdRef = useRef<string | null>(null);
  // `api_ask_ai_run` resolves the id async — events that arrive first are
  // buffered here and replayed (id-filtered) once the invoke resolves.
  const awaitingRunIdRef = useRef(false);
  const bufferedEventsRef = useRef<BufferedEvent[]>([]);
  // Set on unmount. If the page unmounts while `api_ask_ai_run` is still
  // resolving its id, the unmount cleanup can't cancel (no id yet) — the
  // post-await path checks this and cancels the freshly-created run instead.
  const disposedRef = useRef(false);

  useEffect(() => {
  }, []);

  // ⌘K can push '/ask?q=…' while this page is ALREADY mounted — the useState
  // initializer above only runs once, so the input would go stale. Track the
  // last-seen `q` param and adopt it only when it CHANGES to a new non-null
  // value (never clobbering user typing when the param hasn't changed).
  const lastSeenQRef = useRef(searchParams.get('q'));
  useEffect(() => {
    const q = searchParams.get('q');
    if (q !== lastSeenQRef.current) {
      lastSeenQRef.current = q;
      if (q !== null) setQuestion(q);
    }
  }, [searchParams]);

  useEffect(() => {
    void (async () => {
      try {
        const result = await invoke<Person[]>('api_list_people');
        setPeople(Array.isArray(result) ? result : []);
      } catch (err) {
        console.error('[Ask] Failed to load people:', err);
      }
    })();
  }, []);

  const trimmedQuestion = question.trim();
  const scope = useMemo<AggregationScope>(
    () => buildScope({ preset, customFrom, customTo, personId }),
    [preset, customFrom, customTo, personId],
  );

  // WS2 loaders — stable identities (empty deps) so they can sit in event/effect
  // dependency lists without churning subscriptions. Both degrade to [] on failure.
  const refreshHistory = useCallback(async () => {
    setHistory(await listAskAiHistory());
  }, []);
  const refreshSaved = useCallback(async () => {
    setSavedQuestions(await listSavedQuestions());
  }, []);

  useEffect(() => {
    void refreshHistory();
    void refreshSaved();
  }, [refreshHistory, refreshSaved]);

  /** Apply a run event that has already passed the runId filter. */
  const dispatchEvent = useCallback((event: BufferedEvent) => {
    switch (event.kind) {
      case 'progress':
        setProgress(event.payload);
        break;
      case 'complete':
        runIdRef.current = null;
        setCancelling(false);
        setAnswer({ markdown: event.payload.answerMarkdown, sources: event.payload.sources });
        setViewedHistoryId(null); // this is the live answer, not a reloaded one
        setPhase('done');
        // The backend already persisted this run (fire-and-forget); pull it in.
        void refreshHistory();
        break;
      case 'error':
        runIdRef.current = null;
        setCancelling(false);
        if (event.payload.cancelled) {
          // User cancellation — silently back to idle (spec).
          setPhase('idle');
        } else {
          setRunError(event.payload.message);
          setPhase('error');
        }
        break;
    }
  }, [refreshHistory]);

  // Lifetime event subscriptions; unmount cleanup unlistens AND cancels any
  // in-flight run so nothing keeps burning tokens for a page nobody is on.
  useEffect(() => {
    disposedRef.current = false; // StrictMode remount resets the flag
    const accept = (event: BufferedEvent) => {
      if (runIdRef.current === null) {
        if (awaitingRunIdRef.current) bufferedEventsRef.current.push(event);
        return;
      }
      if (event.payload.runId !== runIdRef.current) return; // stale run
      dispatchEvent(event);
    };
    const disposers = [
      safeListen<AskAiProgressPayload>('ask-ai-progress', (e) =>
        accept({ kind: 'progress', payload: e.payload }),
      ),
      safeListen<AskAiCompletePayload>('ask-ai-complete', (e) =>
        accept({ kind: 'complete', payload: e.payload }),
      ),
      safeListen<AskAiErrorPayload>('ask-ai-error', (e) =>
        accept({ kind: 'error', payload: e.payload }),
      ),
    ];
    return () => {
      disposers.forEach((dispose) => dispose());
      disposedRef.current = true; // startRun's post-await path cancels late-resolving ids
      if (runIdRef.current) {
        invoke('api_cancel_ask_ai', { runId: runIdRef.current }).catch((err) => {
          console.error('[Ask] Failed to cancel run on unmount:', err);
        });
        runIdRef.current = null;
      }
    };
  }, [dispatchEvent]);

  const canAsk = phase !== 'running' && trimmedQuestion.length > 0;

  // `override` lets a saved-question re-run (WS2.b) bypass the live input/scope
  // controls and answer against the stored (question, scope) verbatim. With no
  // override this is the normal Ask button / Enter flow reading page state.
  const startRun = useCallback(
    async (override?: { question: string; scope: AggregationScope }) => {
    const runQuestion = (override?.question ?? trimmedQuestion).trim();
    const runScope = override?.scope ?? scope;
    if (!runQuestion || runIdRef.current || awaitingRunIdRef.current) return;
    // Reflect a re-run in the input so the header shows what's being answered.
    if (override) setQuestion(override.question);
    setPhase('running');
    setAnswer(null);
    setViewedHistoryId(null);
    setRunError(null);
    setCancelling(false);
    setProgress({ runId: '', stage: 'gathering', current: 0, total: 0 });
    awaitingRunIdRef.current = true;
    bufferedEventsRef.current = [];
    try {
      const runId = await invoke<string>('api_ask_ai_run', {
        question: runQuestion,
        scope: runScope,
      });
      awaitingRunIdRef.current = false;
      if (disposedRef.current) {
        // Unmounted between click and resolve — kill the run instead of
        // leaking it (cloud egress, tokens). Fire-and-forget.
        invoke('api_cancel_ask_ai', { runId }).catch((err) => {
          console.error('[Ask] Failed to cancel run started before unmount:', err);
        });
        return;
      }
      runIdRef.current = runId;
      const buffered = bufferedEventsRef.current;
      bufferedEventsRef.current = [];
      for (const event of buffered) {
        if (event.payload.runId === runId) dispatchEvent(event);
      }
    } catch (err) {
      // Synchronous config failures (no model chosen, missing API key) — the
      // Rust command returns a user-readable message; show it as the error.
      awaitingRunIdRef.current = false;
      bufferedEventsRef.current = [];
      setRunError(
        typeof err === 'string' ? err : 'Could not start Ask AI. Check your model settings and try again.',
      );
      setPhase('error');
    }
    },
    [trimmedQuestion, scope, dispatchEvent],
  );

  const cancelRun = useCallback(async () => {
    const runId = runIdRef.current;
    if (!runId || cancelling) return;
    setCancelling(true);
    try {
      // `false` just means the run already finished — not an error. The idle
      // transition happens when the cancelled `ask-ai-error` event arrives.
      await invoke<boolean>('api_cancel_ask_ai', { runId });
    } catch (err) {
      console.error('[Ask] Failed to cancel run:', err);
      setCancelling(false);
    }
  }, [cancelling]);

  // --- WS2.a: reload a prior Q&A read-only ---------------------------------
  // Reuses the exact answer/sources render below by populating `answer` from the
  // stored markdown + parsed sources; `viewedHistoryId` marks it as a reload
  // (highlights the row) rather than a fresh live run. No re-answer happens.
  const viewHistoryEntry = useCallback((entry: AskAiHistoryEntry) => {
    if (runIdRef.current) return; // don't clobber an in-flight run
    setQuestion(entry.question);
    setAnswer({ markdown: entry.answerMarkdown, sources: parseSources(entry.sourcesJson) });
    setViewedHistoryId(entry.id);
    setRunError(null);
    setProgress(null);
    setPhase('done');
  }, []);

  // `?historyId=<id>` (or the sentinel `latest`) opens a specific history entry
  // read-only on load — the screenshot pipeline's deep link into an already-answered
  // Ask AI run, mirroring the meeting-details `?tab=` pattern. Applied at most once so
  // it never fights a user's own click or typing afterward.
  const historyDeepLinkAppliedRef = useRef(false);
  useEffect(() => {
    if (historyDeepLinkAppliedRef.current) return;
    const wanted = searchParams.get('historyId');
    if (!wanted || history.length === 0) return;
    const entry = wanted === 'latest' ? history[0] : history.find((h) => h.id === wanted);
    if (!entry) return;
    historyDeepLinkAppliedRef.current = true;
    viewHistoryEntry(entry);
  }, [searchParams, history, viewHistoryEntry]);

  const deleteHistoryEntry = useCallback(
    async (id: string) => {
      try {
        await deleteAskAiHistory(id);
        if (viewedHistoryId === id) setViewedHistoryId(null);
        await refreshHistory();
      } catch (err) {
        toast.error(err instanceof Error ? err.message : 'Could not delete that history entry.');
      }
    },
    [refreshHistory, viewedHistoryId],
  );

  // --- WS2.b: star the current question, re-run / delete a saved one -------
  const saveCurrentQuestion = useCallback(async () => {
    if (!trimmedQuestion || saving) return;
    setSaving(true);
    try {
      // Label defaults to the question text; scope is the SAME object handed to
      // `api_ask_ai_run`, stringified so a re-run round-trips it verbatim.
      await createSavedQuestion(trimmedQuestion, trimmedQuestion, JSON.stringify(scope));
      await refreshSaved();
      toast.success('Question saved');
    } catch (err) {
      toast.error(err instanceof Error ? err.message : 'Could not save that question.');
    } finally {
      setSaving(false);
    }
  }, [trimmedQuestion, saving, scope, refreshSaved]);

  const deleteSavedQuestionEntry = useCallback(
    async (id: string) => {
      try {
        await deleteSavedQuestion(id);
        await refreshSaved();
      } catch (err) {
        toast.error(err instanceof Error ? err.message : 'Could not delete that saved question.');
      }
    },
    [refreshSaved],
  );

  const personLabel =
    personId === null ? 'Anyone' : (people.find((p) => p.id === personId)?.displayName ?? 'Person');

  return (
    <motion.div
      initial={{ opacity: 0, y: 12 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ duration: 0.25, ease: 'easeOut' }}
      className="flex h-page flex-col bg-background"
    >
      <PageHeader
        title="Ask AI"
        subtitle="Ask a question across your meetings — answers cite their sources"
      />

      <div className="flex-1 overflow-y-auto px-7 pb-12">
        <div className="mx-auto max-w-[840px]">
          {/* Question + scope */}
          <div className="rounded-[3px] border border-border bg-card p-4 shadow-sm">
            <input
              type="text"
              value={question}
              onChange={(e) => setQuestion(e.target.value)}
              onKeyDown={(e) => {
                // Enter runs the question directly; IME composition-confirm
                // Enter never submits.
                if (
                  enterTriggersRun({
                    key: e.key,
                    isComposing: e.nativeEvent.isComposing,
                    keyCode: e.keyCode,
                    canAsk,
                  })
                ) {
                  void startRun();
                }
              }}
              placeholder="What did we decide about…"
              aria-label="Question"
              autoFocus
              className="w-full bg-transparent text-[15px] text-foreground outline-none placeholder:text-muted-foreground"
            />

            {/* Scope row: date preset + optional person. Defaults to all
                meetings, ranked by relevance to the question. */}
            <div className="mt-3 flex flex-wrap items-center gap-2">
              <div
                role="group"
                aria-label="Filter by date"
                className="inline-flex items-center rounded-lg border border-border bg-muted p-0.5"
              >
                {DATE_PRESETS.map(({ value, label }) => {
                  const active = preset === value;
                  return (
                    <button
                      key={value}
                      type="button"
                      aria-pressed={active}
                      onClick={() => setPreset(value)}
                      className={cn(
                        'rounded-md px-3 py-1 text-xs font-semibold transition-colors focus:outline-none focus-visible:ring-2 focus-visible:ring-ring',
                        active
                          ? 'bg-card text-foreground shadow-sm'
                          : 'text-muted-foreground hover:text-foreground',
                      )}
                    >
                      {label}
                    </button>
                  );
                })}
              </div>

              {preset === 'custom' && (
                <div className="inline-flex items-center gap-1.5">
                  <input
                    type="date"
                    value={customFrom}
                    onChange={(e) => setCustomFrom(e.target.value)}
                    aria-label="From date"
                    className="h-[30px] rounded-lg border border-border bg-card px-2 text-xs text-foreground focus:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                  />
                  <span className="text-xs text-muted-foreground">to</span>
                  <input
                    type="date"
                    value={customTo}
                    onChange={(e) => setCustomTo(e.target.value)}
                    aria-label="To date"
                    className="h-[30px] rounded-lg border border-border bg-card px-2 text-xs text-foreground focus:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                  />
                </div>
              )}

              <DropdownMenu>
                <DropdownMenuTrigger asChild>
                  <button
                    type="button"
                    aria-label="Filter by person"
                    className="inline-flex h-[30px] items-center gap-1.5 rounded-lg border border-border bg-card px-3 text-xs font-semibold text-foreground transition-colors hover:bg-muted focus:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                  >
                    {personLabel}
                    <ChevronDown size={13} aria-hidden="true" className="text-muted-foreground" />
                  </button>
                </DropdownMenuTrigger>
                <DropdownMenuContent align="start">
                  <DropdownMenuItem onSelect={() => setPersonId(null)}>Anyone</DropdownMenuItem>
                  {people.length > 0 && <DropdownMenuSeparator />}
                  {people.map((p) => (
                    <DropdownMenuItem key={p.id} onSelect={() => setPersonId(p.id)}>
                      {p.displayName}
                    </DropdownMenuItem>
                  ))}
                </DropdownMenuContent>
              </DropdownMenu>
            </div>

            <div className="mt-4 flex flex-wrap items-end justify-between gap-3 border-t border-border pt-3">
              <p className="u-meta min-w-0 flex-1">
                Answers come from your configured summary model and list their sources.
              </p>
              <div className="flex flex-shrink-0 items-center gap-2">
                <button
                  type="button"
                  onClick={() => void saveCurrentQuestion()}
                  disabled={!trimmedQuestion || saving}
                  title="Save this question to re-run later against fresh data"
                  className="inline-flex h-9 items-center gap-1.5 rounded-lg border border-border bg-card px-3 text-sm font-semibold text-foreground transition-colors hover:bg-muted focus:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:cursor-not-allowed disabled:text-muted-foreground"
                >
                  <Star size={14} aria-hidden="true" />
                  Save
                </button>
                <button
                  type="button"
                  onClick={() => void startRun()}
                  disabled={!canAsk}
                  className={cn(
                    'inline-flex h-9 items-center gap-1.5 rounded-lg px-4 text-sm font-semibold transition-colors focus:outline-none focus-visible:ring-2 focus-visible:ring-ring',
                    canAsk
                      ? 'bg-brand text-brand-foreground hover:bg-brand/90'
                      : 'cursor-not-allowed bg-muted text-muted-foreground',
                  )}
                >
                  <Sparkles size={14} aria-hidden="true" />
                  Ask
                </button>
              </div>
            </div>
          </div>

          {/* Run progress */}
          {phase === 'running' && (
            <div className="mt-4 flex items-center justify-between gap-3 rounded-[3px] border border-border bg-card px-4 py-3 shadow-sm">
              <p className="flex min-w-0 items-center gap-2 text-sm text-foreground">
                <Loader2 size={15} aria-hidden="true" className="animate-spin text-brand" />
                <span className="truncate">
                  {progress ? stageLabel(progress) : 'Starting…'}
                </span>
              </p>
              <button
                type="button"
                onClick={() => void cancelRun()}
                disabled={cancelling}
                className="flex-shrink-0 rounded-lg border border-border bg-card px-3 py-1.5 text-xs font-semibold text-foreground transition-colors hover:bg-muted focus:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:cursor-not-allowed disabled:text-muted-foreground"
              >
                {cancelling ? 'Cancelling…' : 'Cancel'}
              </button>
            </div>
          )}

          {/* Error — readable message, input above stays live for a retry. */}
          {phase === 'error' && runError && (
            <div className="mt-4 rounded-[3px] border border-destructive/30 bg-destructive/5 px-4 py-3">
              <p className="text-sm text-foreground">{runError}</p>
              <p className="u-meta mt-1">Adjust the question or your model settings and try again.</p>
            </div>
          )}

          {/* Answer + sources. When `viewedHistoryId` is set the card is a
              read-only reload of a prior Q&A (WS2.a), otherwise the live answer. */}
          {phase === 'done' && answer && (
            <div className="mt-4 rounded-[3px] border border-border bg-card p-6 shadow-sm">
              {viewedHistoryId && (
                <p className="u-meta mb-3 flex items-center gap-1.5 border-b border-border pb-3">
                  <Clock size={12} aria-hidden="true" />
                  Saved answer from your history — ask again to refresh it
                </p>
              )}
              <AnswerMarkdown markdown={answer.markdown} sources={answer.sources} />
              <SourcesList sources={answer.sources} />
            </div>
          )}

          {/* WS2.b — Saved questions: star now, re-run against fresh data later. */}
          {savedQuestions.length > 0 && (
            <section className="mt-8">
              <h2 className="u-section-label mb-2 flex items-center gap-1.5">
                <Star size={12} aria-hidden="true" />
                Saved questions
              </h2>
              <div className="flex flex-col gap-1.5">
                {savedQuestions.map((sq) => (
                  <div
                    key={sq.id}
                    className="group flex items-center gap-2 rounded-lg border border-border bg-card px-3 py-2 shadow-sm"
                  >
                    <button
                      type="button"
                      onClick={() => router.push(`/saved-question?id=${sq.id}`)}
                      title="Open this saved question — shows its last answer, with a Rerun button"
                      className="flex min-w-0 flex-1 items-baseline gap-2 text-left focus:outline-none"
                    >
                      <span className="min-w-0 flex-1 truncate text-[13.5px] font-medium text-foreground group-hover:text-brand">
                        {sq.label?.trim() || sq.question}
                      </span>
                      {sq.lastRunAt && (
                        <span className="flex-shrink-0 text-xs text-muted-foreground">
                          {formatMeetingDate(sq.lastRunAt)}
                        </span>
                      )}
                      <ChevronRight
                        size={13}
                        aria-hidden="true"
                        className="self-center text-muted-foreground group-hover:text-brand"
                      />
                    </button>
                    <button
                      type="button"
                      onClick={() => void deleteSavedQuestionEntry(sq.id)}
                      title="Remove saved question"
                      aria-label="Remove saved question"
                      className="flex-shrink-0 rounded-md p-1.5 text-muted-foreground transition-colors hover:bg-muted hover:text-destructive focus:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                    >
                      <Trash2 size={13} aria-hidden="true" />
                    </button>
                  </div>
                ))}
              </div>
            </section>
          )}

          {/* WS2.a — History: collapsible, most-recent-first, reload read-only. */}
          {history.length > 0 && (
            <section className="mt-8">
              <button
                type="button"
                onClick={() => setHistoryOpen((v) => !v)}
                className="u-section-label mb-2 flex items-center gap-1.5 transition-colors hover:text-foreground focus:outline-none"
                aria-expanded={historyOpen}
              >
                {historyOpen ? (
                  <ChevronDown size={12} aria-hidden="true" />
                ) : (
                  <ChevronRight size={12} aria-hidden="true" />
                )}
                History
                <span className="font-normal normal-case tracking-normal">({history.length})</span>
              </button>
              {historyOpen && (
                <div className="flex flex-col gap-1.5">
                  {history.map((entry) => {
                    const date = formatMeetingDate(entry.createdAt);
                    const active = viewedHistoryId === entry.id;
                    return (
                      <div
                        key={entry.id}
                        className={cn(
                          'group flex items-center gap-2 rounded-lg border bg-card px-3 py-2 shadow-sm transition-colors',
                          active ? 'border-brand/40' : 'border-border',
                        )}
                      >
                        <button
                          type="button"
                          onClick={() => viewHistoryEntry(entry)}
                          title="Reload this answer"
                          className="flex min-w-0 flex-1 items-baseline gap-2 text-left focus:outline-none"
                        >
                          <span
                            className={cn(
                              'min-w-0 flex-1 truncate text-[13.5px]',
                              active
                                ? 'font-semibold text-brand'
                                : 'font-medium text-foreground group-hover:text-brand',
                            )}
                          >
                            {entry.question}
                          </span>
                          {date && (
                            <span className="flex-shrink-0 text-xs text-muted-foreground">{date}</span>
                          )}
                        </button>
                        <button
                          type="button"
                          onClick={() => void deleteHistoryEntry(entry.id)}
                          title="Delete from history"
                          aria-label="Delete from history"
                          className="flex-shrink-0 rounded-md p-1.5 text-muted-foreground transition-colors hover:bg-muted hover:text-destructive focus:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                        >
                          <Trash2 size={13} aria-hidden="true" />
                        </button>
                      </div>
                    );
                  })}
                </div>
              )}
            </section>
          )}
        </div>
      </div>
    </motion.div>
  );
}

export default function AskPage() {
  // useSearchParams requires a Suspense boundary (same pattern as meeting-details).
  return (
    <Suspense
      fallback={
        <div className="flex h-page items-center justify-center bg-background text-muted-foreground">
          <Loader2 className="h-5 w-5 animate-spin" />
        </div>
      }
    >
      <AskPageContent />
    </Suspense>
  );
}
