'use client';

/**
 * To-do shortcuts available on every page:
 *   ⌘T   open the To-do tab (on it already: focus its new-item field). Elsewhere it
 *        navigates with `?add=1`, which the page turns into the same focus once loaded.
 *   ⌘⇧T  quick-add dialog over the current page; while recording, it can link the to-do
 *        to the live meeting and note it in that meeting's My Notes.
 * `openQuickAdd()` opens the same dialog from a button (the record screen's Add To-Do), so
 * there is one dialog and one notion of the live meeting.
 */

import { useEffect, useState } from 'react';
import { usePathname, useRouter } from 'next/navigation';
import { useSidebar } from '@/components/Sidebar/SidebarProvider';
import { useRecordingState } from '@/contexts/RecordingStateContext';
import { useTranscripts } from '@/contexts/TranscriptContext';
import { QuickAddDialog } from './QuickAddDialog';

export const FOCUS_QUICK_ADD_EVENT = 'nixon:focus-quick-add';
export const QUICK_ADD_LABEL = 'New To-Do';
export const OPEN_QUICK_ADD_EVENT = 'nixon:open-quick-add';

/** Open the ⌘⇧T quick-add dialog (mounted once, in the app shell). */
export function openQuickAdd(): void {
  window.dispatchEvent(new Event(OPEN_QUICK_ADD_EVENT));
}

export function QuickAddShortcut() {
  const router = useRouter();
  const pathname = usePathname();
  const [dialogOpen, setDialogOpen] = useState(false);
  const { isRecording } = useRecordingState();
  const { activeRecordingMeetingId } = useSidebar();
  const { meetingTitle } = useTranscripts();
  const liveMeeting =
    isRecording && activeRecordingMeetingId
      ? {
          id: activeRecordingMeetingId,
          title: meetingTitle && meetingTitle !== '+ New Call' ? meetingTitle : 'this Meeting',
        }
      : null;

  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key.toLowerCase() !== 't' || !e.metaKey || e.altKey || e.ctrlKey) return;
      e.preventDefault();
      if (e.shiftKey) {
        setDialogOpen(true);
      } else if (pathname === '/tasks') {
        window.dispatchEvent(new Event(FOCUS_QUICK_ADD_EVENT));
      } else {
        router.push('/tasks?tab=todo&add=1');
      }
    };
    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  }, [pathname, router]);

  useEffect(() => {
    const open = () => setDialogOpen(true);
    window.addEventListener(OPEN_QUICK_ADD_EVENT, open);
    return () => window.removeEventListener(OPEN_QUICK_ADD_EVENT, open);
  }, []);

  return <QuickAddDialog open={dialogOpen} onOpenChange={setDialogOpen} liveMeeting={liveMeeting} />;
}

/** Focus the To-do page's new-item field, if it is on screen. */
export function focusQuickAdd(): boolean {
  const field = document.querySelector<HTMLElement>(`[aria-label="${QUICK_ADD_LABEL}"]`);
  field?.focus();
  return !!field;
}
