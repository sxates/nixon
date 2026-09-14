import { useCallback, useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { toast } from 'sonner';
import { useTranscripts } from '@/contexts/TranscriptContext';
import { useSidebar } from '@/components/Sidebar/SidebarProvider';

/** The sidebar's placeholder title when nothing has named the meeting yet. */
export const TITLE_PLACEHOLDER = '+ New Call';

export interface UseRecordingTitleEditReturn {
  isEditingTitle: boolean;
  titleDraft: string;
  setTitleDraft: (value: string) => void;
  titleInputRef: React.RefObject<HTMLInputElement>;
  startEditingTitle: () => void;
  commitTitleEdit: () => void;
  cancelTitleEdit: () => void;
}

/**
 * Rename-while-recording (specs/0029 WS4.3).
 *
 * Inline click-to-edit title (same interaction pattern as MeetingIdentityHeader:
 * click/pencil to edit, Enter saves, Escape cancels). Saves MUST target the
 * authoritative SQLite row id (`activeRecordingMeetingId`), never the fabricated
 * TranscriptContext `meeting-<timestamp>` id (specs/0024 WS3.1). Editing is
 * disabled until that id exists (the brief window right after pressing Start).
 */
export function useRecordingTitleEdit(): UseRecordingTitleEditReturn {
  const { meetingTitle, setMeetingTitle } = useTranscripts();
  const { refetchMeetings, activeRecordingMeetingId } = useSidebar();

  const [isEditingTitle, setIsEditingTitle] = useState(false);
  const [titleDraft, setTitleDraft] = useState('');
  const titleInputRef = useRef<HTMLInputElement>(null);

  const persistMeetingTitle = useCallback(
    async (title: string) => {
      if (!activeRecordingMeetingId) return;
      try {
        // Backend marks title_manually_set (specs/0024 WS6.1), so the summary
        // auto-title never clobbers a name the user typed here.
        await invoke('api_save_meeting_title', {
          meetingId: activeRecordingMeetingId,
          title,
        });
        void refetchMeetings();
      } catch (error) {
        console.error('Failed to rename meeting:', error);
        toast.error('Failed to rename meeting', {
          description: error instanceof Error ? error.message : String(error),
        });
      }
    },
    [activeRecordingMeetingId, refetchMeetings],
  );

  const startEditingTitle = useCallback(() => {
    setTitleDraft(meetingTitle === TITLE_PLACEHOLDER ? '' : meetingTitle);
    setIsEditingTitle(true);
  }, [meetingTitle]);

  const commitTitleEdit = useCallback(() => {
    setIsEditingTitle(false);
    const trimmed = titleDraft.trim();
    // Don't persist an empty or placeholder title, or a no-op edit.
    if (!trimmed || trimmed === TITLE_PLACEHOLDER || trimmed === meetingTitle.trim()) return;
    // Mirror into TranscriptContext so the header, tray, and stop/save flows all
    // carry the new name immediately.
    setMeetingTitle(trimmed);
    void persistMeetingTitle(trimmed);
  }, [titleDraft, meetingTitle, setMeetingTitle, persistMeetingTitle]);

  const cancelTitleEdit = useCallback(() => {
    setIsEditingTitle(false);
  }, []);

  useEffect(() => {
    if (isEditingTitle) {
      const id = window.setTimeout(() => {
        titleInputRef.current?.focus();
        titleInputRef.current?.select();
      }, 0);
      return () => window.clearTimeout(id);
    }
  }, [isEditingTitle]);

  return {
    isEditingTitle,
    titleDraft,
    setTitleDraft,
    titleInputRef,
    startEditingTitle,
    commitTitleEdit,
    cancelTitleEdit,
  };
}
