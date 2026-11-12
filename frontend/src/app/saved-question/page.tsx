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
import { useRouter, useSearchParams } from 'next/navigation';
import { invoke } from '@tauri-apps/api/core';
import { toast } from 'sonner';
import { Button } from '@/components/ui/button';
import { Callout } from '@/components/ui/callout';
import { DeckIcon } from '@/components/ui/deck-icon';
import { EmptyState, PageLoading } from '@/components/ui/empty-state';
import { ArrowLeft, RefreshCw } from '@/components/ui/icons';
import { Panel } from '@/components/ui/panel';
import { RunStatus } from '@/components/ui/run-status';
import { cn } from '@/lib/utils';
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
import { PAGE_CONTENT_WIDTH, PAGE_GUTTER } from '@/lib/page-width';

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
    return <PageLoading className="bg-background" />;
  }

  if (!saved) {
    return (
      <div className="flex h-page items-center justify-center bg-background px-8">
        <EmptyState
          title="That saved question no longer exists."
          action={
            <Button variant="outline" onClick={() => router.push('/ask')}>
              <DeckIcon icon={ArrowLeft} size={14} />
              Back to Ask AI
            </Button>
          }
        />
      </div>
    );
  }

  const answeredLabel = formatMeetingDate(answeredAt);

  return (
    <div className="flex h-page flex-col bg-background animate-fade-in">
      <PageHeader
        width="page"
        back={{ label: 'Ask AI', onClick: onBack }}
        title={<span className="block min-w-0 truncate">{saved.label?.trim() || saved.question}</span>}
        subtitle="Saved question — shows the last answer; Rerun to refresh it."
        actions={
          <Button
            variant="brand"
            onClick={() => void rerun()}
            disabled={running}
            title="Answer this again against your latest meetings"
          >
            <DeckIcon icon={RefreshCw} size={14} />
            Rerun
          </Button>
        }
      />

      <div className={cn('flex-1 overflow-y-auto pb-12', PAGE_GUTTER)}>
        <div className={cn('mx-auto', PAGE_CONTENT_WIDTH)}>
          {/* Run progress */}
          {running && (
            <RunStatus
              className="mb-4"
              label={progress ? stageLabel(progress) : 'Starting…'}
              onCancel={() => void cancelRun()}
              cancelling={cancelling}
            />
          )}

          {/* Error — readable message; Rerun stays available above. */}
          {phase === 'error' && runError && (
            <Callout tone="error" className="mb-4" title={runError}>
              Adjust your model settings and try Rerun again.
            </Callout>
          )}

          {/* Cached / fresh answer, or the empty state when never run. */}
          {answer ? (
            <Panel size="document">
              {answeredLabel && (
                <p className="u-meta mb-3 border-b border-border pb-3">
                  Last answered {answeredLabel} — Rerun to refresh against your latest meetings
                </p>
              )}
              <AnswerMarkdown markdown={answer.markdown} sources={answer.sources} />
              <SourcesList sources={answer.sources} />
            </Panel>
          ) : (
            !running && (
              <EmptyState
                variant="panel"
                title="Not run yet"
                body="Press Rerun to answer this against your latest meetings. The answer is saved here so opening this page again won't re-run the model."
              />
            )
          )}
        </div>
      </div>
    </div>
  );
}

export default function SavedQuestionPage() {
  // useSearchParams requires a Suspense boundary (same pattern as /ask, meeting-details).
  return (
    <Suspense fallback={<PageLoading className="bg-background" />}>
      <SavedQuestionContent />
    </Suspense>
  );
}
