'use client';

/**
 * Saved-question sub-page (specs/0038 dogfood feedback #4).
 *
 * A saved question used to re-run the LLM every time it was clicked on the Ask page. Now each one
 * has its own page that shows the CACHED answer from the last time it ran — it does NOT auto-run
 * on open (no surprise cloud egress). It only regenerates when the user presses Rerun, which then
 * updates the cache via `updateSavedQuestionAnswer`.
 *
 * The Rerun flow reuses the exact run discipline from /ask (`app/ask/page.tsx`): `api_ask_ai_run`
 * resolves a runId async, the three `ask-ai-*` events are filtered by that id, events that race
 * ahead of the resolve are buffered and replayed, and unmount cancels any in-flight run so a page
 * nobody is on never keeps burning tokens.
 */

import { Suspense, useCallback, useEffect, useRef, useState } from 'react';
import { motion } from 'framer-motion';
import { useRouter, useSearchParams } from 'next/navigation';
import { invoke } from '@tauri-apps/api/core';
import { toast } from 'sonner';
import { ArrowLeft, Clock, Loader2, RotateCw, Sparkles, Star } from 'lucide-react';
import { AnswerMarkdown } from '@/components/AskAI/AnswerMarkdown';
import { SourcesList } from '@/components/AskAI/SourcesList';
import {
  cachedAnswerFor,
  getSavedQuestion,
  parseScope,
  stageLabel,
  updateSavedQuestionAnswer,
  type AskAiCompletePayload,
  type AskAiErrorPayload,
  type AskAiProgressPayload,
  type SavedQuestion,
  type SourceMeeting,
} from '@/lib/ask-ai';
import { formatMeetingDate } from '@/lib/format-date';
import { safeListen } from '@/lib/safe-listen';
import { PageHeader } from '@/components/ui/page-header';

type Phase = 'idle' | 'running' | 'error';

interface Answer {
  markdown: string;
  sources: SourceMeeting[];
}

type BufferedEvent =
  | { kind: 'progress'; payload: AskAiProgressPayload }
  | { kind: 'complete'; payload: AskAiCompletePayload }
  | { kind: 'error'; payload: AskAiErrorPayload };

function SavedQuestionContent() {
  const router = useRouter();
  const searchParams = useSearchParams();
  const id = searchParams.get('id');

  const [loading, setLoading] = useState(true);
  const [saved, setSaved] = useState<SavedQuestion | null>(null);
  // The answer currently on screen — the cached one on load, or a fresh Rerun result.
  const [answer, setAnswer] = useState<Answer | null>(null);
  // ISO of when `answer` was produced (cached `lastRunAt`, or now after a Rerun).
  const [answeredAt, setAnsweredAt] = useState<string | null>(null);

  const [phase, setPhase] = useState<Phase>('idle');
  const [progress, setProgress] = useState<AskAiProgressPayload | null>(null);
  const [runError, setRunError] = useState<string | null>(null);
  const [cancelling, setCancelling] = useState(false);

  // Run-event discipline — identical to /ask (see file header there).
  const runIdRef = useRef<string | null>(null);
  const awaitingRunIdRef = useRef(false);
  const bufferedEventsRef = useRef<BufferedEvent[]>([]);
  const disposedRef = useRef(false);

  // Load the saved question + its cached answer. No LLM call happens here.
  useEffect(() => {
    let alive = true;
    void (async () => {
      if (!id) {
        setLoading(false);
        return;
      }
      const sq = await getSavedQuestion(id);
      if (!alive) return;
      setSaved(sq);
      const cached = sq ? cachedAnswerFor(sq) : null;
      if (cached) {
        setAnswer(cached);
        setAnsweredAt(sq?.lastRunAt ?? null);
      }
      setLoading(false);
    })();
    return () => {
      alive = false;
    };
  }, [id]);

  /** Apply a run event that has already passed the runId filter. */
  const dispatchEvent = useCallback(
    (event: BufferedEvent) => {
      switch (event.kind) {
        case 'progress':
          setProgress(event.payload);
          break;
        case 'complete': {
          runIdRef.current = null;
          setCancelling(false);
          const next: Answer = {
            markdown: event.payload.answerMarkdown,
            sources: event.payload.sources,
          };
          setAnswer(next);
          setAnsweredAt(new Date().toISOString());
          setPhase('idle');
          // Persist the fresh answer as the new cache. Fire-and-forget: a failed
          // write must not lose the answer already on screen.
          if (id) {
            void updateSavedQuestionAnswer(id, next.markdown, JSON.stringify(next.sources)).catch(
              (err) => {
                console.error('[SavedQuestion] Failed to cache answer:', err);
                toast.error('Answered, but could not save it for next time.');
              },
            );
          }
          break;
        }
        case 'error':
          runIdRef.current = null;
          setCancelling(false);
          if (event.payload.cancelled) {
            setPhase('idle');
          } else {
            setRunError(event.payload.message);
            setPhase('error');
          }
          break;
      }
    },
    [id],
  );

  // Lifetime event subscriptions; unmount cancels any in-flight run.
  useEffect(() => {
    disposedRef.current = false;
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
      disposedRef.current = true;
      if (runIdRef.current) {
        invoke('api_cancel_ask_ai', { runId: runIdRef.current }).catch((err) => {
          console.error('[SavedQuestion] Failed to cancel run on unmount:', err);
        });
        runIdRef.current = null;
      }
    };
  }, [dispatchEvent]);

  const rerun = useCallback(async () => {
    if (!saved || runIdRef.current || awaitingRunIdRef.current) return;
    setPhase('running');
    setRunError(null);
    setCancelling(false);
    setProgress({ runId: '', stage: 'gathering', current: 0, total: 0 });
    awaitingRunIdRef.current = true;
    bufferedEventsRef.current = [];
    try {
      const runId = await invoke<string>('api_ask_ai_run', {
        question: saved.question,
        scope: parseScope(saved.scopeJson),
      });
      awaitingRunIdRef.current = false;
      if (disposedRef.current) {
        invoke('api_cancel_ask_ai', { runId }).catch((err) => {
          console.error('[SavedQuestion] Failed to cancel run started before unmount:', err);
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
      awaitingRunIdRef.current = false;
      bufferedEventsRef.current = [];
      setRunError(
        typeof err === 'string'
          ? err
          : 'Could not start the run. Check your model settings and try again.',
      );
      setPhase('error');
    }
  }, [saved, dispatchEvent]);

  const cancelRun = useCallback(async () => {
    const runId = runIdRef.current;
    if (!runId || cancelling) return;
    setCancelling(true);
    try {
      await invoke<boolean>('api_cancel_ask_ai', { runId });
    } catch (err) {
      console.error('[SavedQuestion] Failed to cancel run:', err);
      setCancelling(false);
    }
  }, [cancelling]);

  const onBack = useCallback(() => {
    if (window.history.length <= 1) {
      router.push('/ask');
    } else {
      router.back();
    }
  }, [router]);

  const running = phase === 'running';

  if (loading) {
    return (
      <div className="flex h-page items-center justify-center bg-background text-muted-foreground">
        <Loader2 className="h-5 w-5 animate-spin" />
      </div>
    );
  }

  if (!saved) {
    return (
      <div className="flex h-page flex-col items-center justify-center gap-4 bg-background px-8 text-center">
        <p className="text-sm text-muted-foreground">That saved question no longer exists.</p>
        <button
          type="button"
          onClick={() => router.push('/ask')}
          className="inline-flex h-9 items-center gap-1.5 rounded-lg border border-border bg-card px-3 text-sm font-semibold text-foreground transition-colors hover:bg-muted focus:outline-none focus-visible:ring-2 focus-visible:ring-ring"
        >
          <ArrowLeft size={14} aria-hidden="true" />
          Back to Ask AI
        </button>
      </div>
    );
  }

  const answeredLabel = formatMeetingDate(answeredAt);

  return (
    <motion.div
      initial={{ opacity: 0, y: 12 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ duration: 0.25, ease: 'easeOut' }}
      className="flex h-page flex-col bg-background"
    >
      <div className="flex-shrink-0 px-4 min-[900px]:px-7 pt-7">
        <button
          type="button"
          onClick={onBack}
          className="u-meta inline-flex items-center gap-1.5 text-muted-foreground transition-colors hover:text-foreground focus:outline-none"
        >
          <ArrowLeft size={14} aria-hidden="true" />
          Ask AI
        </button>
      </div>
      <PageHeader
        className="pt-3"
        title={
          <span className="flex items-center gap-2">
            <Star size={18} aria-hidden="true" className="flex-shrink-0 text-brand" />
            <span className="min-w-0 truncate">{saved.label?.trim() || saved.question}</span>
          </span>
        }
        subtitle="Saved question — shows the last answer; Rerun to refresh it."
        actions={
          <button
            type="button"
            onClick={() => void rerun()}
            disabled={running}
            title="Answer this again against your latest meetings"
            className="inline-flex h-9 flex-shrink-0 items-center gap-1.5 rounded-lg bg-brand px-4 text-sm font-semibold text-brand-foreground transition-colors hover:bg-brand/90 focus:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:cursor-not-allowed disabled:bg-muted disabled:text-muted-foreground"
          >
            <RotateCw size={14} aria-hidden="true" />
            Rerun
          </button>
        }
      />

      <div className="flex-1 overflow-y-auto px-4 min-[900px]:px-7 pb-12">
        <div className="mx-auto max-w-[840px]">
          {/* Run progress */}
          {running && (
            <div className="mb-4 flex items-center justify-between gap-3 rounded-[3px] border border-border bg-card px-4 py-3 shadow-sm">
              <p className="flex min-w-0 items-center gap-2 text-sm text-foreground">
                <Loader2 size={15} aria-hidden="true" className="animate-spin text-brand" />
                <span className="truncate">{progress ? stageLabel(progress) : 'Starting…'}</span>
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

          {/* Error — readable message; Rerun stays available above. */}
          {phase === 'error' && runError && (
            <div className="mb-4 rounded-[3px] border border-destructive/30 bg-destructive/5 px-4 py-3">
              <p className="text-sm text-foreground">{runError}</p>
              <p className="u-meta mt-1">Adjust your model settings and try Rerun again.</p>
            </div>
          )}

          {/* Cached / fresh answer, or the empty state when never run. */}
          {answer ? (
            <div className="rounded-[3px] border border-border bg-card p-6 shadow-sm">
              {answeredLabel && (
                <p className="u-meta mb-3 flex items-center gap-1.5 border-b border-border pb-3">
                  <Clock size={12} aria-hidden="true" />
                  Last answered {answeredLabel} — Rerun to refresh against your latest meetings
                </p>
              )}
              <AnswerMarkdown markdown={answer.markdown} sources={answer.sources} />
              <SourcesList sources={answer.sources} />
            </div>
          ) : (
            !running && (
              <div className="rounded-[3px] border border-dashed border-border bg-card/50 px-6 py-10 text-center">
                <Sparkles size={20} aria-hidden="true" className="mx-auto mb-3 text-muted-foreground" />
                <p className="text-sm font-medium text-foreground">Not run yet</p>
                <p className="u-meta mx-auto mt-1 max-w-sm">
                  Press Rerun to answer this against your latest meetings. The answer is saved here
                  so opening this page again won&apos;t re-run the model.
                </p>
              </div>
            )
          )}
        </div>
      </div>
    </motion.div>
  );
}

export default function SavedQuestionPage() {
  // useSearchParams requires a Suspense boundary (same pattern as /ask, meeting-details).
  return (
    <Suspense
      fallback={
        <div className="flex h-page items-center justify-center bg-background text-muted-foreground">
          <Loader2 className="h-5 w-5 animate-spin" />
        </div>
      }
    >
      <SavedQuestionContent />
    </Suspense>
  );
}
