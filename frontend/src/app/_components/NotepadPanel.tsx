'use client';

import { useCallback, useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { toast } from 'sonner';
import { useSidebar } from '@/components/Sidebar/SidebarProvider';
import { NoteEditor } from '@/components/NoteEditor/NoteEditor';
import { htmlToMarkdown, markdownToHtml, sanitizeNotesHtml, looksLikeHtml } from '@/lib/notes-html';

/** Debounce interval (ms) for autosaving notes after the user stops typing. */
const AUTOSAVE_DEBOUNCE_MS = 1000;

/** Sentinel meeting id used by SidebarProvider for the "+ New Call" placeholder (no DB row yet). */
const PLACEHOLDER_MEETING_ID = 'intro-call';

/**
 * Subset of `api_get_meeting_notes` we read (serde camelCase; see database/models.rs).
 * `notesMarkdown` is the canonical store consumed by the summary pipeline; `notesJson`
 * round-trips the editor HTML for faithful reload (legacy rows hold BlockNote JSON there,
 * which we detect and fall back to markdown for).
 */
interface MeetingNotesResponse {
  meetingId: string;
  notesMarkdown: string | null;
  notesJson: string | null;
}

/**
 * Returns the real (DB-backed) meeting id, or null when there isn't one yet.
 *
 * With persist-at-start (spec 0007) the meeting row is created the moment recording begins, so
 * currentMeeting.id is real for the whole session. The only time we still see the placeholder
 * is on the recorder before a recording has started — there's nothing to save to yet.
 */
function resolveMeetingId(id: string | undefined | null): string | null {
  if (!id || id === PLACEHOLDER_MEETING_ID) return null;
  return id;
}

interface NotepadPanelProps {
  /**
   * Explicit meeting id to bind to: the `?id=` URL param in the post-meeting view, or the
   * live recording's id (`activeRecordingMeetingId`) on the recording screen. When omitted,
   * the id is resolved from the sidebar's currentMeeting — a VIEWED-meeting tracker, so only
   * safe as a fallback for the pre-recording window where no meeting row exists yet
   * (spec 0029 WS5.1).
   */
  meetingId?: string;
  /** Whether to render the "My notes" section label (recording screen). Off in tabbed views. */
  showHeader?: boolean;
  /**
   * "bare" makes the notepad blend into the surrounding document column (transparent
   * background, no card chrome) for the meeting-details "My notes" tab. Default "panel"
   * keeps the card background used by the live recording screen.
   */
  variant?: 'panel' | 'bare';
}

/**
 * Live notepad pane. Used both on the recording screen (explicit id of the live recording)
 * and in the post-meeting "My Notes" tab (explicit id from the URL).
 *
 * - Loads any existing notes for the meeting via `api_get_meeting_notes`.
 * - Debounced autosave (markdown for the summary + HTML for reload) via `api_save_meeting_notes`.
 * - Every save targets the meeting the current editor content was loaded for, so notes can
 *   never be written under a different meeting than the one they were typed into (0029 WS5.1).
 * - No-ops gracefully in the brief window before a recording has started (no DB id yet).
 */
export function NotepadPanel({
  meetingId: meetingIdProp,
  showHeader = true,
  variant = 'panel',
}: NotepadPanelProps) {
  const { currentMeeting } = useSidebar();
  // An explicit prop takes precedence; otherwise resolve from the sidebar (recording screen).
  const meetingId = meetingIdProp ?? resolveMeetingId(currentMeeting?.id);

  // Latest editor HTML (kept in a ref so the debounced/flush save always reads current content).
  const htmlRef = useRef<string>('');
  // Initial HTML for the visible editor once notes have loaded (or '' for a blank notepad).
  const [initialHtml, setInitialHtml] = useState<string | null>(null);
  // Bumped to force-remount the editor when we load a different meeting's notes.
  const [editorKey, setEditorKey] = useState(0);
  const saveTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

  // Track which meeting id we currently have loaded, so a transition between meetings can flush
  // any in-memory edits before we reload for the new id.
  const loadedMeetingIdRef = useRef<string | null>(null);
  // Whether the visible editor has been given its first content yet.
  const hasInitializedRef = useRef(false);

  // Serialize the current notes and persist them. No-ops when there's no DB meeting id yet.
  const saveNotes = useCallback(async (targetMeetingId: string | null) => {
    if (!targetMeetingId) return;

    const html = htmlRef.current;
    const markdown = htmlToMarkdown(html);

    try {
      await invoke('api_save_meeting_notes', {
        meetingId: targetMeetingId,
        notesMarkdown: markdown || null,
        notesJson: html.trim() ? html : null,
      });
    } catch (error) {
      // Don't crash the screen if a save fails; surface a quiet, friendly toast.
      console.error('Failed to autosave meeting notes:', error);
      toast.error('Could not save your notes', {
        description: 'Your typing is kept on screen; we will retry on the next change.',
        duration: 4000,
      });
    }
  }, []);

  // Load notes whenever the resolved meeting id changes.
  useEffect(() => {
    let cancelled = false;

    const previousMeetingId = loadedMeetingIdRef.current;

    const loadNotes = async () => {
      if (previousMeetingId !== meetingId) {
        // Switching away from a loaded meeting: flush any pending (debounced) edits to the
        // meeting they belong to BEFORE loading the new one, so they can't be saved under
        // the wrong id later (spec 0029 WS5.1).
        if (previousMeetingId && saveTimer.current) {
          clearTimeout(saveTimer.current);
          saveTimer.current = null;
          await saveNotes(previousMeetingId);
        }

        // Transitioning from "no DB meeting" to a real meeting id (e.g. user typed before a
        // recording started): flush what's already in the editor so those notes aren't lost
        // on reload.
        if (!previousMeetingId && meetingId && htmlRef.current.trim()) {
          await saveNotes(meetingId);
        }
      }

      if (!meetingId) {
        // No persisted meeting yet: start (or keep) a blank notepad.
        loadedMeetingIdRef.current = null;
        if (!hasInitializedRef.current && !cancelled) {
          hasInitializedRef.current = true;
          htmlRef.current = '';
          setInitialHtml('');
        } else if (previousMeetingId && !cancelled) {
          // A meeting WAS loaded (e.g. the recording just ended): blank the editor so its
          // content can't be flushed into the NEXT meeting on a later null → id transition.
          htmlRef.current = '';
          setInitialHtml('');
          setEditorKey((k) => k + 1);
        }
        return;
      }

      try {
        const notes = await invoke<MeetingNotesResponse | null>('api_get_meeting_notes', {
          meetingId,
        });
        if (cancelled) return;

        // Prefer our round-trip HTML; fall back to rendering markdown (incl. legacy notes
        // whose notesJson holds BlockNote JSON, which isn't HTML).
        let html = '';
        if (looksLikeHtml(notes?.notesJson)) {
          html = sanitizeNotesHtml(notes!.notesJson!);
        } else if (notes?.notesMarkdown) {
          html = markdownToHtml(notes.notesMarkdown);
        }

        htmlRef.current = html;
        hasInitializedRef.current = true;
        setInitialHtml(html);
        setEditorKey((k) => k + 1); // remount editor so loaded content renders
        loadedMeetingIdRef.current = meetingId;
      } catch (error) {
        console.error('Failed to load meeting notes:', error);
        if (!cancelled) {
          toast.error('Could not load saved notes', {
            description: 'Starting with a blank notepad for this meeting.',
            duration: 4000,
          });
          htmlRef.current = '';
          hasInitializedRef.current = true;
          setInitialHtml('');
          loadedMeetingIdRef.current = meetingId;
        }
      }
    };

    loadNotes();

    return () => {
      cancelled = true;
    };
  }, [meetingId, saveNotes]);

  // Flush any pending autosave on unmount. Previously this only cleared the debounce timer,
  // silently dropping notes typed within the last AUTOSAVE_DEBOUNCE_MS before the panel
  // unmounted (navigating away / stopping a recording) — spec 0028, High. Now we fire the
  // save first, then clear the timer. The flush targets the meeting the current editor
  // content was LOADED for (never a re-resolved global, which can point at a different
  // meeting by unmount time — spec 0029 WS5.1). saveNotes is stable (no deps), so this
  // stays mount-only.
  useEffect(() => {
    return () => {
      if (saveTimer.current) {
        clearTimeout(saveTimer.current);
        saveTimer.current = null;
        void saveNotes(loadedMeetingIdRef.current);
      }
    };
  }, [saveNotes]);

  const handleChange = useCallback(
    (html: string) => {
      htmlRef.current = html;

      if (saveTimer.current) {
        clearTimeout(saveTimer.current);
      }
      saveTimer.current = setTimeout(() => {
        // Save to the meeting whose notes are in the editor — NOT a re-resolved global id,
        // which can change out from under a live recording (spec 0029 WS5.1). While no
        // meeting exists yet this is null (no-op); the load effect flushes the editor
        // content once a real id appears.
        saveNotes(loadedMeetingIdRef.current);
      }, AUTOSAVE_DEBOUNCE_MS);
    },
    [saveNotes],
  );

  const isBare = variant === 'bare';
  const label = !isBare && showHeader ? 'My notes' : undefined;

  if (initialHtml === null) {
    // Brief loading window — keep layout stable, no spinner chrome.
    return <div className={`min-h-0 w-full flex-1 ${isBare ? '' : 'bg-card'}`} />;
  }

  return (
    <NoteEditor
      key={editorKey}
      initialHtml={initialHtml}
      onChange={handleChange}
      label={label}
      className={`min-h-0 w-full flex-1 ${isBare ? '' : 'bg-card px-5 pb-3 pt-3'}`}
      bodyClassName={isBare ? 'pb-10' : 'pb-8'}
    />
  );
}
