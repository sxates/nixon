'use client';

import { useCallback, useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { ListChecks } from 'lucide-react';
import { AnswerMarkdown } from '@/components/AskAI/AnswerMarkdown';
import { safeListen } from '@/lib/safe-listen';
import type { PrepNotes } from '@/lib/prep';

/**
 * Pinned, read-only agenda for the live recording (specs/0036). When the meeting being
 * recorded has prep notes ("what I plan to cover"), they stay in view above the live
 * notepad so the agenda is glanceable during the call. Renders nothing when there are no
 * prep notes, so an ad-hoc recording is visually unchanged. Reuses AnswerMarkdown for the
 * small markdown vocabulary (no citations here — sources is empty).
 */
export function RecordAgendaPanel({ meetingId }: { meetingId: string }) {
  const [markdown, setMarkdown] = useState<string | null>(null);

  const load = useCallback(async () => {
    try {
      const notes = await invoke<PrepNotes | null>('api_get_prep_notes', { meetingId });
      const md = notes?.prepMarkdown?.trim() ? notes.prepMarkdown : null;
      setMarkdown(md);
    } catch (error) {
      console.warn('Could not load prep agenda for the recording:', error);
    }
  }, [meetingId]);

  useEffect(() => {
    void load();
  }, [load]);

  // A background prep pass (or an edit on the Prep tab) can change the agenda — refresh.
  useEffect(() => safeListen('prep-briefs-updated', () => void load()), [load]);

  if (!markdown) return null;

  return (
    <div className="flex max-h-[38%] flex-shrink-0 flex-col overflow-hidden border-b border-border bg-muted/40">
      <div className="flex items-center gap-1.5 px-5 pt-3 text-[11px] font-semibold uppercase tracking-[0.05em] text-muted-foreground">
        <ListChecks size={12} aria-hidden="true" />
        Agenda — what you planned to cover
      </div>
      <div className="min-h-0 overflow-y-auto px-5 pb-3 pt-1.5">
        <AnswerMarkdown markdown={markdown} sources={[]} />
      </div>
    </div>
  );
}
