/**
 * Appending a "☐ To-do: …" marker line to a meeting's My Notes (specs/0088 quick-add).
 *
 * The line records that a to-do was made during the meeting; it is a plain marker, not a
 * live checkbox. The `☐ To-do:` prefix is load-bearing: the action-item extractor skips
 * lines that start with it (the to-do already exists), so it is not suggested again.
 *
 * Two paths, so a save never clobbers the other:
 *   - The meeting's notes are open in a `NotepadPanel`: it takes the line into the live
 *     editor (and its own autosave persists it) and marks the event handled.
 *   - Otherwise the line is appended to the stored notes directly.
 */

import { invoke } from '@tauri-apps/api/core';
import { htmlToMarkdown, looksLikeHtml, markdownToHtml, sanitizeNotesHtml } from '@/lib/notes-html';

export const TODO_NOTE_PREFIX = '☐ To-do:';
export const APPEND_NOTE_LINE_EVENT = 'nixon:append-note-line';

export interface AppendNoteLineDetail {
  meetingId: string;
  line: string;
  /** Set by the panel that took the line. */
  handled: boolean;
}

/** `☐ To-do: Send the deck (due Friday)` — `due` is a display label, e.g. "Friday". */
export function todoNoteLine(title: string, due?: string | null): string {
  return `${TODO_NOTE_PREFIX} ${title.trim()}${due ? ` (due ${due})` : ''}`;
}

const escapeHtml = (s: string) =>
  s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');

/** `html` with `line` as a new last paragraph. */
export function appendParagraphHtml(html: string, line: string): string {
  return `${html.trim()}<p>${escapeHtml(line)}</p>`;
}

export async function appendLineToMeetingNotes(meetingId: string, line: string): Promise<void> {
  const detail: AppendNoteLineDetail = { meetingId, line, handled: false };
  window.dispatchEvent(new CustomEvent<AppendNoteLineDetail>(APPEND_NOTE_LINE_EVENT, { detail }));
  if (detail.handled) return;

  const notes = await invoke<{ notesMarkdown: string | null; notesJson: string | null } | null>(
    'api_get_meeting_notes',
    { meetingId },
  );
  let html = '';
  if (looksLikeHtml(notes?.notesJson)) html = sanitizeNotesHtml(notes!.notesJson!);
  else if (notes?.notesMarkdown) html = markdownToHtml(notes.notesMarkdown);
  const next = appendParagraphHtml(html, line);
  await invoke('api_save_meeting_notes', {
    meetingId,
    notesMarkdown: htmlToMarkdown(next) || null,
    notesJson: next,
  });
}
