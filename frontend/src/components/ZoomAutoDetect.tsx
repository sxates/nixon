'use client';

/**
 * Zoom meeting auto-detection (spec 0008, P1).
 *
 * Connects the backend Zoom monitor's Tauri events to the recording flow:
 *   - `zoom-meeting-detected` → fire a NATIVE macOS notification ("Zoom meeting
 *     detected" / "Record this meeting?") with [Record] / [Ignore] action
 *     buttons, so the user sees it even when Nixon is backgrounded (they're in
 *     back-to-back meetings). On Record (button or body-click), start recording
 *     via the EXISTING sidebar start path (handleRecordingToggle →
 *     autoStartRecording flag + navigate to /record), which creates the meeting +
 *     captures audio via persist-at-start. A body-click also focuses the Nixon
 *     window first. We never auto-start on a timeout (v1 = explicit confirm).
 *
 *     If the OS notification can't be sent (plugin/permission unavailable —
 *     common on a bare `cargo run` dev binary; actionable macOS notifications
 *     generally need a bundled `.app`), we fall back to the in-app sonner toast
 *     so nothing is lost. When the window is already focused we ALSO show the
 *     toast as a convenient in-app affordance. Either way it's ONE prompt per
 *     detected meeting.
 *   - `zoom-meeting-ended` → if Nixon is currently recording, auto-stop via the
 *     SAME full two-part stop as "Stop & summarize" (`requestFullRecordingStop`,
 *     specs/0024 WS1.1): the backend `stop_recording` (stops the audio tap +
 *     flushes the WAV) then post-stop processing/summary. If not recording, do
 *     nothing. (Previously called `window.handleRecordingStop` directly, which
 *     skipped `stop_recording` and left the tap running.)
 *
 * Mounted inside the provider tree in layout.tsx so it is global (works on any
 * route, even when the window is backgrounded). Listeners use `safeListen` so
 * teardown is race-safe / crash-proof.
 *
 * The backend only emits `zoom-meeting-detected` on the Idle→InMeeting
 * transition (and only when Nixon isn't already recording + the feature is on),
 * so we get at most one prompt per meeting — ignoring it won't re-prompt.
 */

import { useEffect, useRef } from 'react';
import { useRouter } from 'next/navigation';
import { toast } from 'sonner';
import { safeListen } from '@/lib/safe-listen';
import { notify, focusMainWindow } from '@/lib/osNotification';
import { requestFullRecordingStop } from '@/lib/recording-stop';
import { peekPendingJoinMeeting } from '@/lib/calendar';
import { useRecordingState } from '@/contexts/RecordingStateContext';
import { useSidebar } from '@/components/Sidebar/SidebarProvider';

interface ZoomMeetingEvent {
  timestamp_ms: number;
  kind: 'detected' | 'ended';
}

// Stable toast id so a stale "detected" prompt is replaced rather than stacked
// if the backend ever re-emits before the previous one is dismissed.
const DETECTED_TOAST_ID = 'zoom-meeting-detected';

export default function ZoomAutoDetect() {
  const { isRecording } = useRecordingState();
  const { handleRecordingToggle } = useSidebar();
  const router = useRouter();

  // Keep the latest values in refs so the long-lived event listeners (set up
  // once on mount) always read current state without re-subscribing.
  const isRecordingRef = useRef(isRecording);
  const handleRecordingToggleRef = useRef(handleRecordingToggle);
  const routerRef = useRef(router);
  useEffect(() => {
    isRecordingRef.current = isRecording;
  }, [isRecording]);
  useEffect(() => {
    handleRecordingToggleRef.current = handleRecordingToggle;
  }, [handleRecordingToggle]);
  useEffect(() => {
    routerRef.current = router;
  }, [router]);

  // True when a Zoom meeting was detected but the user has NOT yet been shown a
  // prompt they can act on — specifically when the OS notification couldn't be
  // delivered while the window was unfocused, so the prompt was lost. When the
  // window regains focus we surface the in-app toast for any still-pending
  // detection. Cleared once the user is prompted/records/ignores or the meeting
  // ends.
  const pendingUnpromptedRef = useRef(false);

  // specs/0029 WS1.3: the "Record this meeting?" toast is persistent (Infinity) and
  // was dismissed only by its own buttons or zoom-meeting-ended — so it lingered when
  // a recording started by any other path (tray, sidebar, Join & Record's delayed
  // start). Dismiss it the moment a recording begins, however it began.
  useEffect(() => {
    if (isRecording) {
      toast.dismiss(DETECTED_TOAST_ID);
      pendingUnpromptedRef.current = false;
    }
  }, [isRecording]);

  useEffect(() => {
    // Start recording for the detected meeting, re-checking state at click time
    // (the user may have started recording manually since the prompt appeared).
    // handleRecordingToggle itself no-ops while recording, so this fires once.
    const startRecording = () => {
      pendingUnpromptedRef.current = false;
      if (isRecordingRef.current) {
        console.log('[ZoomAutoDetect] Already recording; ignoring Record action');
        toast.dismiss(DETECTED_TOAST_ID);
        return;
      }
      console.log('[ZoomAutoDetect] User chose to record detected meeting');
      // Reuse the existing start path (autoStartRecording flag + navigate to
      // /record), same as the previous in-app toast did.
      handleRecordingToggleRef.current();
      toast.dismiss(DETECTED_TOAST_ID);
    };

    // In-app sonner toast — used (a) as a fallback when the OS notification
    // can't be delivered, and (b) as an extra affordance when the window is
    // already focused. Identical Record/Ignore semantics to the OS notification.
    const showToast = () => {
      // The user now has an actionable prompt in front of them.
      pendingUnpromptedRef.current = false;
      toast('Zoom meeting detected', {
        id: DETECTED_TOAST_ID,
        description: 'Record this meeting?',
        duration: Infinity, // persistent — explicit confirm only
        action: {
          label: 'Record',
          onClick: startRecording,
        },
        cancel: {
          label: 'Ignore',
          onClick: () => {
            console.log('[ZoomAutoDetect] User ignored detected meeting');
            pendingUnpromptedRef.current = false;
            toast.dismiss(DETECTED_TOAST_ID);
          },
        },
      });
    };

    const disposeDetected = safeListen<ZoomMeetingEvent>('zoom-meeting-detected', () => {
      console.log('[ZoomAutoDetect] Zoom meeting detected');

      // Defensive: backend only emits when not already recording, but guard
      // anyway so a race can never produce a double-start prompt.
      if (isRecordingRef.current) {
        console.log('[ZoomAutoDetect] Already recording, ignoring detection');
        return;
      }

      // specs/0029 WS2.1: a Join & Record is armed — opening Zoom is exactly what
      // tripped this detection. Its delayed start fires shortly and adopts the
      // stashed calendar identity; a competing "Record?" prompt here could only
      // race it into a second, mistitled row. Stay silent.
      if (peekPendingJoinMeeting()) {
        console.log('[ZoomAutoDetect] Join & Record pending; suppressing detection prompt');
        return;
      }

      // If the window is already focused, the in-app toast is the nicer
      // affordance — show it directly and skip the OS notification (keeps it to
      // one prompt). Otherwise, fire the native notification so a backgrounded
      // user still sees it.
      // ALWAYS show the persistent in-app Record/Ignore toast — this is the reliable
      // actionable prompt. We intentionally do NOT depend on macOS notification action
      // buttons: they only render in "Alerts" notification style (new apps default to
      // "Banners"), and the body-click action routing is flaky. So the toast is the
      // source of truth for the choice, and it's always present in the app.
      showToast();

      // If Nixon is backgrounded, ALSO fire an OS notification so the user is alerted
      // when they aren't looking at the app. Clicking it brings Nixon to the front,
      // where the Record/Ignore toast is waiting. The Record/Ignore action buttons are
      // still attached for users whose macOS notification style is set to "Alerts".
      const windowFocused = typeof document !== 'undefined' && document.hasFocus();
      console.log('[ZoomAutoDetect] handling detection; windowFocused =', windowFocused);
      if (!windowFocused) {
        void notify({
          title: 'Zoom meeting detected',
          body: 'Record this meeting? Open Nixon to choose.',
          onRecord: startRecording,
          onIgnore: () => {
            console.log('[ZoomAutoDetect] User ignored detected meeting (OS notification)');
            toast.dismiss(DETECTED_TOAST_ID);
          },
          onClick: () => {
            // Body click reliably activates the app — bring it forward so the
            // persistent Record/Ignore toast is visible and the user can choose.
            console.log('[ZoomAutoDetect] OS notification clicked; focusing (toast awaits)');
            void focusMainWindow();
          },
        });
      }
    });

    // When the window regains focus, surface any detection prompt that was lost
    // because the OS notification couldn't be delivered while we were backgrounded.
    const onFocus = () => {
      if (!pendingUnpromptedRef.current) return;
      if (isRecordingRef.current) {
        pendingUnpromptedRef.current = false;
        return;
      }
      console.log('[ZoomAutoDetect] Window refocused with pending detection; showing toast');
      showToast();
    };
    window.addEventListener('focus', onFocus);

    const disposeEnded = safeListen<ZoomMeetingEvent>('zoom-meeting-ended', () => {
      console.log('[ZoomAutoDetect] Zoom meeting ended');

      // Dismiss any still-open "detected" prompt for this meeting and clear any
      // pending-unprompted detection (the meeting is over — nothing to prompt).
      toast.dismiss(DETECTED_TOAST_ID);
      pendingUnpromptedRef.current = false;

      if (!isRecordingRef.current) {
        // Not recording — nothing to wrap up.
        return;
      }

      // Treat Zoom-end EXACTLY like the user pressing "Stop & summarize" (specs/0024 WS1.1):
      // run the FULL two-part stop (backend `stop_recording` to stop the audio tap + flush the
      // WAV, THEN post-stop processing/summary). Previously this called
      // `window.handleRecordingStop(true)` directly, which only ran post-processing and left the
      // Core Audio tap recording — so the recording widget lingered, a later manual stop created a
      // spurious second meeting, and the duration was inflated by the post-call capture.
      console.log('[ZoomAutoDetect] Auto-stopping recording (meeting ended)');
      toast('Meeting ended', { description: 'Wrapping up your recording…' });
      requestFullRecordingStop((path) => routerRef.current.push(path));
    });

    return () => {
      disposeDetected();
      disposeEnded();
      window.removeEventListener('focus', onFocus);
    };
  }, []);

  return null;
}
