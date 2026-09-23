'use client';

/**
 * The "a call started — record it?" prompt (spec 0008, rebuilt in specs/0074 W5).
 *
 * On the backend's detected event it shows TWO prompts, deliberately:
 *   - the in-app Record/Ignore toast, which survives a refused notification permission; and
 *   - an OS banner (`CATEGORY_RECORD`), ALWAYS — focused or not. The delegate presents
 *     banners while Nixon is frontmost too, and the record category persists until acted on
 *     (0070 W3), so the banner is the prompt that is still there after you switch to the call.
 * Acting on either takes the other down, so the question is never left asked twice. The
 * banner also comes down when the meeting ends or a recording starts by any other path.
 *
 * When the banner could not be sent (permission refused, delivery error), the toast says so
 * once per session, with an Enable action to the notification permission row. An unbundled
 * dev build cannot notify at all and has nothing to enable, so it stays quiet about it.
 *
 * On the ended event: if Nixon is recording, run the SAME full two-part stop as "Stop &
 * summarize" (`requestFullRecordingStop`, specs/0024 WS1.1); otherwise do nothing.
 *
 * The events keep their `zoom-meeting-*` names for now; the payload's optional `platform`
 * (absent = Zoom) names the app in the copy. The backend emits `detected` only on the
 * idle → in-meeting transition, while not recording and with the setting on.
 */

import { useCallback, useEffect, useRef } from 'react';
import { useRouter } from 'next/navigation';
import { toast } from 'sonner';
import { safeListen } from '@/lib/safe-listen';
import {
  notify,
  removeNotification,
  focusMainWindow,
  getNotificationCapability,
  CATEGORY_RECORD,
} from '@/lib/osNotification';
import { requestFullRecordingStop } from '@/lib/recording-stop';
import { peekPendingJoinMeeting } from '@/lib/calendar';
import {
  type MeetingDetectEvent,
  detectedTitle,
  detectedBannerId,
  DETECTED_BODY,
  NOTIFICATIONS_OFF_COPY,
  NOTIFICATION_SETTINGS_ROUTE,
} from '@/lib/meeting-detect';
import { useRecordingState } from '@/contexts/RecordingStateContext';
import { useSidebar } from '@/components/Sidebar/SidebarProvider';

// Stable toast id so a stale prompt is replaced rather than stacked.
const DETECTED_TOAST_ID = 'meeting-detected';

export default function MeetingAutoDetect() {
  const { isRecording } = useRecordingState();
  const { handleRecordingToggle } = useSidebar();
  const router = useRouter();

  // Latest values for the long-lived listeners (set up once on mount).
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

  // The current prompt: bumped per detection, so a notify() that resolves after the prompt
  // was answered (or replaced) can tell and take its own banner back down.
  const promptGenRef = useRef(0);
  const promptOpenRef = useRef(false);
  // The delivered banner twin of the open toast, if any.
  const bannerIdRef = useRef<string | null>(null);
  // "Notifications are off" is said once per session, not on every call.
  const failureShownRef = useRef(false);

  /** Take both prompts down: the toast and its banner twin. */
  const closePrompt = useCallback(() => {
    promptOpenRef.current = false;
    toast.dismiss(DETECTED_TOAST_ID);
    const bannerId = bannerIdRef.current;
    bannerIdRef.current = null;
    if (bannerId) void removeNotification(bannerId);
  }, []);

  // specs/0029 WS1.3: a recording started by any path answers the prompt.
  useEffect(() => {
    if (isRecording) closePrompt();
  }, [isRecording, closePrompt]);

  useEffect(() => {
    // Re-check at click time: the user may have started recording since the prompt showed.
    // handleRecordingToggle itself no-ops while recording, so this fires once.
    const startRecording = () => {
      const alreadyRecording = isRecordingRef.current;
      closePrompt();
      if (alreadyRecording) {
        console.log('[MeetingAutoDetect] Already recording; ignoring Record action');
        return;
      }
      console.log('[MeetingAutoDetect] User chose to record detected meeting');
      handleRecordingToggleRef.current();
    };

    const showToast = (title: string, notificationsOff: boolean) => {
      toast(title, {
        id: DETECTED_TOAST_ID,
        description: notificationsOff ? (
          <span>
            {DETECTED_BODY} {NOTIFICATIONS_OFF_COPY}{' '}
            <button
              type="button"
              className="font-medium underline underline-offset-2"
              onClick={() => routerRef.current.push(NOTIFICATION_SETTINGS_ROUTE)}
            >
              Enable
            </button>
          </span>
        ) : (
          DETECTED_BODY
        ),
        duration: Infinity, // persistent — explicit confirm only
        action: { label: 'Record', onClick: startRecording },
        cancel: {
          label: 'Ignore',
          onClick: () => {
            console.log('[MeetingAutoDetect] User ignored detected meeting');
            closePrompt();
          },
        },
      });
    };

    const onDetected = async (event: MeetingDetectEvent | null | undefined) => {
      // Defensive: the backend only emits when not recording.
      if (isRecordingRef.current) return;
      // specs/0029 WS2.1: a Join & Record is armed — opening the call is what tripped this,
      // and its delayed start adopts the calendar identity. A competing prompt could only
      // race it into a second, mistitled row.
      if (peekPendingJoinMeeting()) {
        console.log('[MeetingAutoDetect] Join & Record pending; suppressing detection prompt');
        return;
      }

      // A new call replaces a stale prompt: its banner goes, and the toast is updated in
      // place under the same id — not dismissed first, since sonner merges a re-created id
      // into the dismissed entry and it would vanish with it.
      const staleBanner = bannerIdRef.current;
      bannerIdRef.current = null;
      if (staleBanner) void removeNotification(staleBanner);
      const gen = ++promptGenRef.current;
      promptOpenRef.current = true;
      const title = detectedTitle(event?.platform);
      showToast(title, false);

      const bannerId = detectedBannerId(event);
      const delivered = await notify({
        title,
        body: DETECTED_BODY,
        category: CATEGORY_RECORD,
        id: bannerId,
        onRecord: startRecording,
        // Tapped the banner body, not the button: bring Nixon forward, where the toast waits.
        onOpen: () => void focusMainWindow(),
      });
      const stillOpen = () => promptOpenRef.current && promptGenRef.current === gen;

      if (delivered) {
        if (stillOpen()) bannerIdRef.current = bannerId;
        else void removeNotification(bannerId); // answered in-app while we were sending
        return;
      }

      if (failureShownRef.current) return;
      const { supported } = await getNotificationCapability();
      if (!supported || !stillOpen()) return;
      failureShownRef.current = true;
      console.warn('[MeetingAutoDetect] OS banner not delivered; telling the user');
      showToast(title, true);
    };

    const disposeDetected = safeListen<MeetingDetectEvent>('zoom-meeting-detected', (event) => {
      console.log('[MeetingAutoDetect] Meeting detected');
      void onDetected(event?.payload);
    });

    const disposeEnded = safeListen<MeetingDetectEvent>('zoom-meeting-ended', () => {
      console.log('[MeetingAutoDetect] Meeting ended');
      // The meeting is over: nothing left to ask.
      closePrompt();
      if (!isRecordingRef.current) return;

      // Same full two-part stop as "Stop & summarize" (specs/0024 WS1.1): backend
      // `stop_recording` (stops the tap, flushes the WAV), then post-stop processing.
      console.log('[MeetingAutoDetect] Auto-stopping recording (meeting ended)');
      toast('Meeting ended', { description: 'Wrapping up your recording…' });
      requestFullRecordingStop((path) => routerRef.current.push(path));
    });

    return () => {
      disposeDetected();
      disposeEnded();
    };
  }, [closePrompt]);

  return null;
}
