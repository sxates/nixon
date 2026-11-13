'use client';

/**
 * My notes (specs/0089 §UI): the owner's own notes on a topic. No AI path ever writes them
 * and this card has no AI affordance. Autosave mirrors the meeting notepad
 * (`NotepadPanel`): 1000 ms after typing stops, markdown plus the editor HTML, and a
 * pending save flushes on unmount. The editor is uncontrolled, so the parent keys this
 * card by topic id; a refresh never reseeds text the owner is typing.
 */

import { useCallback, useEffect, useRef, useState } from 'react';
import { toast } from 'sonner';
import { NoteEditor } from '@/components/NoteEditor/NoteEditor';
import { Panel } from '@/components/ui/panel';
import { htmlToMarkdown, looksLikeHtml, markdownToHtml, sanitizeNotesHtml } from '@/lib/notes-html';
import { setTopicNotes } from '@/lib/topics';
import type { Topic } from '@/types';

export const TOPIC_NOTES_DEBOUNCE_MS = 1000;

function seedHtml(topic: Topic): string {
  if (looksLikeHtml(topic.notesJson)) return sanitizeNotesHtml(topic.notesJson!);
  return topic.notesMarkdown ? markdownToHtml(topic.notesMarkdown) : '';
}

export function TopicNotesCard({ topic }: { topic: Topic }) {
  const topicId = topic.id;
  // Seed once: later refreshes must not reset the uncontrolled editor.
  const [initialHtml] = useState(() => seedHtml(topic));
  const htmlRef = useRef(initialHtml);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);

  const save = useCallback(async () => {
    const html = htmlRef.current;
    try {
      await setTopicNotes(topicId, htmlToMarkdown(html), html.trim() ? html : null);
    } catch (e) {
      console.error('Failed to save topic notes:', e);
      toast.error('Could not save your notes', {
        description: 'Your typing is kept on screen; we will retry on the next change.',
        duration: 4000,
      });
    }
  }, [topicId]);

  // Flush a pending save on unmount, so navigating away mid-debounce keeps the text.
  const saveRef = useRef(save);
  saveRef.current = save;
  useEffect(
    () => () => {
      if (timer.current) {
        clearTimeout(timer.current);
        timer.current = null;
        void saveRef.current();
      }
    },
    [],
  );

  const onChange = useCallback(
    (html: string) => {
      if (html === htmlRef.current) return; // blur re-emits unchanged content
      htmlRef.current = html;
      if (timer.current) clearTimeout(timer.current);
      timer.current = setTimeout(() => {
        timer.current = null;
        void save();
      }, TOPIC_NOTES_DEBOUNCE_MS);
    },
    [save],
  );

  return (
    <NoteEditor
      initialHtml={initialHtml}
      onChange={onChange}
      ariaLabel="My notes"
      placeholder="Anything you want to remember about this topic"
      bodyClassName="min-h-16 pb-2"
      frame={({ toolbar, body }) => (
        <Panel title="My notes" action={toolbar} className="flex min-h-0 flex-col">
          <p className="u-row-meta -mt-1 pb-2">Yours. AI never rewrites this.</p>
          {body}
        </Panel>
      )}
    />
  );
}
