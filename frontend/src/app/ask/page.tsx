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
import { ChevronDown, ChevronRight, Loader2, Sparkles, Star, Trash2 } from 'lucide-react';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu';
import { AnswerMarkdown } from '@/components/AskAI/AnswerMarkdown';
import { CopyAnswerButton } from '@/components/AskAI/CopyAnswerButton';
import { SourcesList } from '@/components/AskAI/SourcesList';
import {
  buildScope,
  customRangeProblem,
  scopeLabel,
  createSavedQuestion,
  deleteAskAiHistory,
  deleteSavedQuestion,
  enterTriggersRun,
  listAskAiHistory,
  listSavedQuestions,
  parseScope,
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
  { value: '7d', label: '7d' },
  { value: '30d', label: '30d' },
  { value: 'all', label: 'All time' },
  { value: 'custom', label: 'Custom' },
];

/**
 * Owner feedback 2026-09-21: "'All time' is probably too broad of a default time frame,
 * lets make the default 7d." It also makes every first answer cheaper — the scope bounds
 * how many meetings the engine gathers before it calls the model.
 */
const DEFAULT_PRESET: DatePreset = '7d';

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
  const [preset, setPreset] = useState<DatePreset>(DEFAULT_PRESET);
  const [customFrom, setCustomFrom] = useState('');
  const [customTo, setCustomTo] = useState('');
  const [personId, setPersonId] = useState<string | null>(null);
  const [people, setPeople] = useState<Person[]>([]);

  const [phase, setPhase] = useState<Phase>('idle');
  const [progress, setProgress] = useState<AskAiProgressPayload | null>(null);
  const [answer, setAnswer] = useState<Answer | null>(null);
  // The scope the live answer was produced under — shown beside it, since the controls
  // above can change after the run.
  const [answerScope, setAnswerScope] = useState<AggregationScope>({});
  const [runError, setRunError] = useState<string | null>(null);
  const [cancelling, setCancelling] = useState(false);

  // WS2 persistence side-panels — additive to the live single-question flow.
  const [history, setHistory] = useState<AskAiHistoryEntry[]>([]);
  const [historyOpen, setHistoryOpen] = useState(false);
  const [savedQuestions, setSavedQuestions] = useState<SavedQuestion[]>([]);
  const [saving, setSaving] = useState(false);
  // Which history entry is expanded in place, or null. Owner feedback 2026-09-21: "in the
  // history, I'd like to expand each historical question in-line with the answer, instead of
  // having it replace the question/answer area at the top." An accordion — one open at a
  // time — so a long history never becomes a wall of answers.
  const [expandedHistoryId, setExpandedHistoryId] = useState<string | null>(null);

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

  const rangeProblem = customRangeProblem(preset, customFrom, customTo);
  const canAsk = phase !== 'running' && trimmedQuestion.length > 0 && !rangeProblem;

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
    setAnswerScope(runScope);
    // Collapse any open history row: a new run is about to fill the card above, and two
    // answers on screen with no indication which is which is the confusion this replaced.
    setExpandedHistoryId(null);
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

  // --- WS2.a: read a prior Q&A in place ------------------------------------
  // It used to load the stored answer into the TOP card, which made that card mean two
  // different things and needed a banner to say which. Now the row opens under itself and
  // the top card is only ever the question you just asked (owner feedback 2026-09-21).
  //
  // Nothing here touches the run state, so expanding a history entry mid-run is harmless —
  // it no longer has anything to clobber.
  const toggleHistoryEntry = useCallback((entry: AskAiHistoryEntry) => {
    setExpandedHistoryId((current) => (current === entry.id ? null : entry.id));
  }, []);

  // Re-ask a past question with the question box, so it runs against today's meetings and
  // lands in the live card like any other run.
  const askAgain = useCallback(
    (entry: AskAiHistoryEntry) => {
      setQuestion(entry.question);
      setExpandedHistoryId(null);
      // The CURRENT scope, not the one the answer was produced under: "ask again" means
      // "against what I'm looking at now". Re-running a stored scope verbatim is what a
      // saved question is for.
      void startRun({ question: entry.question, scope });
    },
    [scope, startRun],
  );

  // `?historyId=<id>` (or the sentinel `latest`) opens a specific history entry on load —
  // the screenshot pipeline's deep link into an already-answered Ask AI run, mirroring the
  // meeting-details `?tab=` pattern. Since the row expands in place rather than filling the
  // top card, this also opens the History section so the target is actually on screen.
  // Applied at most once so it never fights a user's own click or typing afterward.
  const historyDeepLinkAppliedRef = useRef(false);
  useEffect(() => {
    if (historyDeepLinkAppliedRef.current) return;
    const wanted = searchParams.get('historyId');
    if (!wanted || history.length === 0) return;
    const entry = wanted === 'latest' ? history[0] : history.find((h) => h.id === wanted);
    if (!entry) return;
    historyDeepLinkAppliedRef.current = true;
    setHistoryOpen(true);
    setExpandedHistoryId(entry.id);
  }, [searchParams, history]);

  const deleteHistoryEntry = useCallback(
    async (id: string) => {
      try {
        await deleteAskAiHistory(id);
        if (expandedHistoryId === id) setExpandedHistoryId(null);
        await refreshHistory();
      } catch (err) {
        toast.error(err instanceof Error ? err.message : 'Could not delete that history entry.');
      }
    },
    [refreshHistory, expandedHistoryId],
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

  // Owner feedback 2026-09-21: "I'm not sure how the 'who' filter works - does that search
  // only transcript segments from a particular speaker?" It does not, and the label was the
  // whole problem. `AggregationScope.person_id` selects MEETINGS the person was in — on the
  // roster (`meeting_participants`) or actually speaking (`speakers.person_id`), union
  // semantics — and then the question is answered over those whole meetings. So the control
  // says so.
  const personName = personId === null ? null : people.find((p) => p.id === personId)?.displayName;
  const personLabel =
    personId === null ? 'Any meeting' : `Meetings with ${personName ?? 'this person'}`;

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

            {/* Owner feedback 2026-09-21: "Input at the very top, full width, but then
                below that Ask/Save on the left, then the filter options on the right in the
                same area." The actions and the scope used to be on two separate rows with a
                rule between them, which put Ask a long way from the thing it acts on. */}
            <div className="mt-3 flex flex-wrap items-center justify-between gap-x-4 gap-y-2">
              <div className="flex flex-shrink-0 items-center gap-2">
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
              </div>

              {/* Scope: which meetings the question may draw from. */}
              <div className="flex flex-wrap items-center justify-end gap-2">
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
                    {rangeProblem && <span className="text-xs text-muted-foreground">{rangeProblem}</span>}
                  </div>
                )}

                <DropdownMenu>
                  <DropdownMenuTrigger asChild>
                    <button
                      type="button"
                      aria-label="Filter by the meetings someone was in"
                      title="Answer from meetings this person was in — on the invite or actually speaking. It does not narrow the answer to their lines."
                      className="inline-flex h-[30px] max-w-[15rem] items-center gap-1.5 rounded-lg border border-border bg-card px-3 text-xs font-semibold text-foreground transition-colors hover:bg-muted focus:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                    >
                      <span className="truncate">{personLabel}</span>
                      <ChevronDown size={13} aria-hidden="true" className="flex-shrink-0 text-muted-foreground" />
                    </button>
                  </DropdownMenuTrigger>
                  <DropdownMenuContent align="end">
                    <DropdownMenuItem onSelect={() => setPersonId(null)}>Any meeting</DropdownMenuItem>
                    {people.length > 0 && <DropdownMenuSeparator />}
                    {people.map((p) => (
                      <DropdownMenuItem key={p.id} onSelect={() => setPersonId(p.id)}>
                        Meetings with {p.displayName}
                      </DropdownMenuItem>
                    ))}
                  </DropdownMenuContent>
                </DropdownMenu>
              </div>
            </div>

            <p className="u-meta mt-3 border-t border-border pt-2.5">
              Answers come from your configured summary model and list their sources.
            </p>
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

          {/* The answer to the question you just asked. Owner feedback 2026-09-21 made this
              card live-run-only: a history entry now expands where it sits, so this card
              stopped being two things at once and lost the "Saved answer from your history"
              banner that existed to tell them apart. */}
          {phase === 'done' && answer && (
            <div className="mt-4 rounded-[3px] border border-border bg-card p-6 shadow-sm">
              <div className="mb-3 flex items-center justify-between gap-2">
                <span className="u-meta">{scopeLabel(answerScope)}</span>
                <CopyAnswerButton markdown={answer.markdown} />
              </div>
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
                    const open = expandedHistoryId === entry.id;
                    return (
                      <div
                        key={entry.id}
                        className={cn(
                          'group rounded-lg border bg-card shadow-sm transition-colors',
                          open ? 'border-brand/40' : 'border-border',
                        )}
                      >
                        <div className="flex items-center gap-2 px-3 py-2">
                          <button
                            type="button"
                            onClick={() => toggleHistoryEntry(entry)}
                            aria-expanded={open}
                            title={open ? 'Collapse this answer' : 'Show this answer'}
                            className="flex min-w-0 flex-1 items-baseline gap-2 text-left focus:outline-none"
                          >
                            <ChevronRight
                              size={13}
                              aria-hidden="true"
                              className={cn(
                                'self-center flex-shrink-0 text-muted-foreground transition-transform',
                                open && 'rotate-90',
                              )}
                            />
                            <span
                              className={cn(
                                'min-w-0 flex-1 truncate text-[13.5px]',
                                open
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
                        {/* The answer, in place. Sources included — the citation chips are
                            the point of an Ask AI answer, and Copy strips them for you. */}
                        {open && (
                          <div className="border-t border-border px-4 pb-4 pt-3">
                            <div className="mb-2 flex items-center justify-end gap-2">
                              <span className="u-meta mr-auto">
                                {scopeLabel(parseScope(entry.scopeJson))}
                              </span>
                              <CopyAnswerButton markdown={entry.answerMarkdown} />
                              <button
                                type="button"
                                onClick={() => askAgain(entry)}
                                disabled={phase === 'running' || !!rangeProblem}
                                title="Ask this again against your current meetings"
                                className="inline-flex h-7 flex-shrink-0 items-center gap-1.5 rounded-lg border border-border bg-card px-2.5 text-xs font-semibold text-muted-foreground transition-colors hover:bg-muted hover:text-foreground focus:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:cursor-not-allowed disabled:text-muted-foreground/50"
                              >
                                <Sparkles size={13} aria-hidden="true" />
                                Ask again
                              </button>
                            </div>
                            <AnswerMarkdown
                              markdown={entry.answerMarkdown}
                              sources={parseSources(entry.sourcesJson)}
                            />
                            <SourcesList sources={parseSources(entry.sourcesJson)} />
                          </div>
                        )}
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
