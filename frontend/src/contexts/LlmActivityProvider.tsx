'use client';

import { createContext, useCallback, useContext, useEffect, useState, type ReactNode } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { safeListen } from '@/lib/safe-listen';

export type LlmTaskKind =
  | 'prepBrief'
  | 'meetingSummary'
  | 'actionItems'
  | 'noteEnhancement'
  | 'askAI'
  | 'rollup'
  // Not an LLM task, but the registry tracks it too (specs/0063 W3) — an offline
  // diarization pass, surfaced in the same queue as everything else the machine is busy with.
  | 'diarization';

export interface RunningTask {
  id: number;
  kind: LlmTaskKind;
  label: string;
  note: string | null;
  meetingId: string | null;
}

/**
 * A background task waiting for its turn (specs/0074 W3/W4) — e.g. a prep brief the pass
 * planned but hasn't started yet. Foreground tasks never appear here (`Priority::Interactive`
 * work is not queued the same way). Oldest first.
 */
export interface QueuedTask {
  id: number;
  kind: LlmTaskKind;
  label: string;
  meetingId: string | null;
}

/**
 * Terminal outcome of a finished task (specs/0053 W3 added `skipped`). Mirrors the Rust
 * `TaskOutcome` enum's `#[serde(tag = "type")]` shape. A skip is deliberate, healthy
 * behaviour — e.g. action-item extraction skipped because the outline found no
 * commitments — and must render distinctly from `failed`, never as an error.
 */
export type TaskOutcome =
  | { type: 'success' }
  | { type: 'failed'; error: string }
  | { type: 'skipped'; reason: string };

export interface TaskRecord {
  id: number;
  kind: LlmTaskKind;
  label: string;
  /**
   * null on success or skip; the error message on failure. Kept for older call sites;
   * `outcome` is the source of truth for rendering.
   */
  error: string | null;
  meetingId: string | null;
  outcome: TaskOutcome;
}

export interface LlmActivityView {
  queued: QueuedTask[];
  running: RunningTask[];
  history: TaskRecord[];
  hasFailure: boolean;
}

const EMPTY: LlmActivityView = { queued: [], running: [], history: [], hasFailure: false };

export interface LlmActivityValue extends LlmActivityView {
  dismiss: () => Promise<void>;
}

const LlmActivityContext = createContext<LlmActivityValue | null>(null);

/**
 * Background LLM activity (spec 0052). Mounted once at app level.
 *
 * Pulls a snapshot on mount and then follows `llm-activity-changed`, so the indicator is
 * correct even if the frontend mounted after a transition it never saw — and a missed event
 * self-heals on the next transition.
 *
 * Every call is best-effort and swallows its error: the indicator exists to REPORT failures,
 * so it must never become a source of them (a toast here would be noise about noise).
 */
export function LlmActivityProvider({ children }: { children: ReactNode }) {
  const [view, setView] = useState<LlmActivityView>(EMPTY);

  useEffect(() => {
    let cancelled = false;

    invoke<LlmActivityView>('api_llm_activity_snapshot')
      .then((snapshot) => {
        if (!cancelled) setView(snapshot);
      })
      .catch(() => {
        /* best-effort: an unavailable snapshot just leaves the row idle */
      });

    const unlisten = safeListen<LlmActivityView>('llm-activity-changed', (event) => {
      setView(event.payload);
    });

    return () => {
      cancelled = true;
      unlisten();
    };
  }, []);

  const dismiss = useCallback(async () => {
    try {
      await invoke('api_llm_activity_dismiss');
    } catch {
      /* best-effort */
    }
  }, []);

  return (
    <LlmActivityContext.Provider value={{ ...view, dismiss }}>
      {children}
    </LlmActivityContext.Provider>
  );
}

export function useLlmActivity(): LlmActivityValue {
  const ctx = useContext(LlmActivityContext);
  if (!ctx) throw new Error('useLlmActivity must be used within an LlmActivityProvider');
  return ctx;
}

/** Like `useLlmActivity` but returns null instead of throwing outside the provider. */
export function useOptionalLlmActivity(): LlmActivityValue | null {
  return useContext(LlmActivityContext);
}
