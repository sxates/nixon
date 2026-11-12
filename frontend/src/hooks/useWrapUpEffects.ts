'use client';

/**
 * specs/0083 W2b — the webview's side of a meeting wrap-up, mounted once at the app root.
 *
 * Rust saves and processes the meeting; this only tidies up the UI around it:
 *   - `saved` / `discarded`: mark the IndexedDB recovery copy saved, refetch the meetings,
 *     clear the live recording id, and clear the live transcript (or, while the recorder is
 *     on screen, clear it when you leave, so it doesn't blank under you);
 *   - `saved`, when the window is visible and not already on that meeting: a
 *     "Meeting saved — processing" toast whose View opens it;
 *   - any stage's `toast` (transcription incomplete, a refused handoff, a skipped summary),
 *     once per event; `failed`: an error toast.
 *
 * Navigation: only from the recorder. If the window is on `/record` (watching the live
 * transcript) when the meeting is saved, it moves to that meeting's summary (2026-11-11).
 * Anywhere else stays put — the owner was pulled out of another meeting or prep whenever a
 * recording ended (2026-11-05).
 */

import { useEffect, useRef } from 'react';
import { usePathname, useRouter } from 'next/navigation';
import { toast } from 'sonner';
import { safeListen } from '@/lib/safe-listen';
import { useTranscripts } from '@/contexts/TranscriptContext';
import { useSidebar } from '@/components/Sidebar/SidebarProvider';
import {
  WRAP_UP_EVENT,
  SAVED_TOAST_TITLE,
  meetingHref,
  meetingSummaryHref,
  shouldOpenSummaryOnSave,
  shouldShowSavedToast,
  type WrapUpState,
} from '@/lib/wrap-up';

/** Sentinel meeting id SidebarProvider uses for the "+ New Call" placeholder. */
const PLACEHOLDER_MEETING_ID = 'intro-call';

function isVisible(): boolean {
  return typeof document === 'undefined' || document.visibilityState === 'visible';
}

export function useWrapUpEffects(): void {
  const { markMeetingAsSaved, clearTranscripts } = useTranscripts();
  const {
    refetchMeetings,
    setActiveRecordingMeetingId,
    setIsMeetingActive,
    currentMeeting,
    setCurrentMeeting,
  } = useSidebar();
  const router = useRouter();
  const pathname = usePathname();

  // The listener registers once; everything it calls is read through this ref.
  const deps = useRef({
    markMeetingAsSaved,
    clearTranscripts,
    refetchMeetings,
    setActiveRecordingMeetingId,
    setIsMeetingActive,
    currentMeeting,
    setCurrentMeeting,
    router,
    pathname,
  });
  deps.current = {
    markMeetingAsSaved,
    clearTranscripts,
    refetchMeetings,
    setActiveRecordingMeetingId,
    setIsMeetingActive,
    currentMeeting,
    setCurrentMeeting,
    router,
    pathname,
  };

  // Set when a wrap-up finishes while the recorder is on screen: its transcript stays until
  // you leave `/record` (or the next recording clears it).
  const clearOnLeave = useRef(false);
  useEffect(() => {
    if (clearOnLeave.current && pathname !== '/record') {
      clearOnLeave.current = false;
      deps.current.clearTranscripts();
    }
  }, [pathname]);

  useEffect(() => {
    const finishRecording = async (state: WrapUpState) => {
      const d = deps.current;
      d.setActiveRecordingMeetingId(null);
      d.setIsMeetingActive(false);
      if (d.pathname === '/record') clearOnLeave.current = true;
      else d.clearTranscripts();
      if (state.stage === 'discarded' && d.currentMeeting?.id === state.meetingId) {
        d.setCurrentMeeting({ id: PLACEHOLDER_MEETING_ID, title: '+ New Call' });
      }
      try {
        await d.markMeetingAsSaved();
      } catch (error) {
        console.warn('[wrap-up] could not mark the recovery copy saved:', error);
      }
      try {
        await d.refetchMeetings();
      } catch (error) {
        console.warn('[wrap-up] could not refresh the meetings list:', error);
      }
    };

    const onWrapUp = (state: WrapUpState) => {
      if (state.stage === 'saved' || state.stage === 'discarded') void finishRecording(state);

      if (state.stage === 'saved' && state.meetingId) {
        const meetingId = state.meetingId;
        if (shouldOpenSummaryOnSave(window.location.pathname)) {
          // `replace`: the emptied recorder isn't worth a Back step.
          deps.current.router.replace(meetingSummaryHref(meetingId));
        } else if (
          shouldShowSavedToast({
            visible: isVisible(),
            pathname: window.location.pathname,
            search: window.location.search,
            meetingId,
          })
        ) {
          toast.success(SAVED_TOAST_TITLE, {
            description: state.title ?? undefined,
            action: {
              label: 'View',
              onClick: () => deps.current.router.push(meetingHref(meetingId)),
            },
          });
        }
      }

      if (state.stage === 'failed') {
        deps.current.setActiveRecordingMeetingId(null);
        toast.error('Couldn’t finish saving the meeting', {
          description: state.message ?? 'Your recording is still on disk. Please try again.',
          duration: 10000,
        });
      }

      // Rust's own copy for this stage (incomplete transcript, refused handoff, skipped
      // summary). Cleared on the next stage, so each one shows once.
      if (state.toast) {
        const show = state.stage === 'summary-skipped' ? toast.info : toast.warning;
        show(state.toast.title, { description: state.toast.description, duration: 10000 });
      }
    };

    return safeListen<WrapUpState>(WRAP_UP_EVENT, (event) => {
      if (event.payload) onWrapUp(event.payload);
    });
  }, []);
}

/** Mounts `useWrapUpEffects` inside the providers it reads. */
export function WrapUpEffects(): null {
  useWrapUpEffects();
  return null;
}
