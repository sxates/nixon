'use client';

/**
 * PrepNotesEditor (specs/0036) — the "things I plan to cover" agenda editor on the
 * Prep tab. Mirrors NotepadPanel's editor + debounced-autosave pattern, but persists
 * to the DEDICATED prep-notes store (`api_save_prep_notes`) so an agenda never leaks
 * into the live `notes_markdown` the summary treats as spoken notes.
 *
 * Content round-trips as HTML (for faithful reload) and is converted to markdown for
 * the summary's "intended agenda" block. The editor is uncontrolled: it seeds once
 * from `initialMarkdown`/`initialJson` (supplied by `api_get_prep`), so a later
 * PrepView refetch never yanks the caret. The parent keys the Prep tab per meeting,
 * so there is no in-place meeting switch to reconcile here.
 */

import { useCallback, useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { toast } from 'sonner';
import { NoteEditor } from '@/components/NoteEditor/NoteEditor';
import { htmlToMarkdown, markdownToHtml, sanitizeNotesHtml, looksLikeHtml } from '@/lib/notes-html';

/** Debounce interval (ms) for autosaving prep notes after the user stops typing. */
const AUTOSAVE_DEBOUNCE_MS = 1000;

interface PrepNotesEditorProps {
  meetingId: string;
  /** Seed markdown from `api_get_prep` (used when no round-trip HTML is stored). */
  initialMarkdown: string | null;
  /** Round-trip HTML if present; falls back to rendering `initialMarkdown`. */
  initialJson: string | null;
}

export function PrepNotesEditor({ meetingId, initialMarkdown, initialJson }: PrepNotesEditorProps) {
  // Seed the editor HTML exactly once (prefer round-trip HTML, else render markdown).
  const [initialHtml] = useState<string>(() => {
    if (looksLikeHtml(initialJson)) return sanitizeNotesHtml(initialJson!);
    if (initialMarkdown) return markdownToHtml(initialMarkdown);
    return '';
  });

  // Latest editor HTML (ref so the debounced/flush save always reads current content).
  const htmlRef = useRef<string>(initialHtml);
  const saveTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

  const save = useCallback(async () => {
    const html = htmlRef.current;
    const markdown = htmlToMarkdown(html);
    try {
      await invoke('api_save_prep_notes', {
        meetingId,
        prepMarkdown: markdown || null,
        prepJson: html.trim() ? html : null,
      });
    } catch (error) {
      console.error('Failed to autosave prep notes:', error);
      toast.error('Could not save your prep notes', {
        description: 'Your typing is kept on screen; we will retry on the next change.',
        duration: 4000,
      });
    }
  }, [meetingId]);

  // Flush any pending autosave on unmount (navigating away / switching tabs never
  // silently drops the last edits typed inside the debounce window).
  useEffect(() => {
    return () => {
      if (saveTimer.current) {
        clearTimeout(saveTimer.current);
        saveTimer.current = null;
        void save();
      }
    };
  }, [save]);

  const handleChange = useCallback(
    (html: string) => {
      htmlRef.current = html;
      if (saveTimer.current) clearTimeout(saveTimer.current);
      saveTimer.current = setTimeout(() => {
        void save();
      }, AUTOSAVE_DEBOUNCE_MS);
    },
    [save],
  );

  return (
    <NoteEditor
      initialHtml={initialHtml}
      onChange={handleChange}
      label="Prep notes — what I plan to cover"
      placeholder="What do you want to cover? Jot your agenda — it stays in view during the call and shapes the summary."
      className="rounded-lg border border-border bg-card px-4 pb-3 pt-3"
      bodyClassName="min-h-[180px] pb-4"
    />
  );
}
