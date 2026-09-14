'use client';

import { useEffect, useRef } from 'react';
import { safeListen } from '@/lib/safe-listen';

/**
 * Owns the recording-screen transcription event listeners
 * (`transcription-error`, `speech-detected`).
 *
 * These used to live inside `RecordingControls`. Because that component is rendered in two
 * different positions on `/record` (a floating pill before recording, in-header controls during
 * recording), the not-recording→recording transition unmounted one instance and mounted the
 * other — leaving a brief window where NO listener was registered, during which an event fired
 * at exactly that moment (e.g. a `transcription-error` as recording starts) would be dropped and
 * no error alert would surface.
 *
 * Hoisting the listeners here fixes that: this hook is called from `record/page.tsx`, which stays
 * mounted for the entire lifetime of the screen, so registration is continuous and gap-free —
 * and there is exactly one registration (no double-listen).
 *
 * The handlers are kept in refs so the listeners register exactly ONCE (empty dep array) and
 * never re-register when the caller's callbacks change identity between renders.
 */
export function useTranscriptionErrorListeners({
  onRecordingStop,
}: {
  /** Same `onRecordingStop` the recording controls receive; called with `false` on error. */
  onRecordingStop: (callApi?: boolean) => void;
}) {
  // Hold the latest callback in a ref so the listener effect can run once and stay stable.
  const onRecordingStopRef = useRef(onRecordingStop);
  useEffect(() => {
    onRecordingStopRef.current = onRecordingStop;
  }, [onRecordingStop]);

  useEffect(() => {
    console.log('Setting up recording event listeners (page-level)');

    // Each safeListen returns a synchronous, idempotent, crash-proof cleanup, so there is no
    // async race between unmount and listener registration.
    const disposers: (() => void)[] = [];

    // Transcription error listener - handles structured error objects with an actionable flag.
    disposers.push(
      safeListen('transcription-error', (event) => {
        console.log('transcription-error event received:', event);
        console.error('Transcription error received:', event.payload);

        console.log('Calling onRecordingStop(false) due to transcription error');
        onRecordingStopRef.current(false);

        // For actionable errors (like model loading failures), the page handles showing the model
        // selector; for regular errors, useModalState's global listener shows a toast — so this
        // hook deliberately raises no modal of its own (matching RecordingControls' behaviour).
      }),
    );

    // Speech detected listener - VAD feedback. Currently has no UI effect, but kept so the event
    // continues to be consumed exactly as before.
    disposers.push(
      safeListen('speech-detected', (event) => {
        console.log('speech-detected event received:', event);
      }),
    );

    console.log('Recording event listeners set up successfully (page-level)');

    return () => {
      console.log('Cleaning up recording event listeners (page-level)');
      disposers.forEach((dispose) => dispose());
    };
    // Register once for the lifetime of the screen; callbacks are read from refs.
  }, []);
}
