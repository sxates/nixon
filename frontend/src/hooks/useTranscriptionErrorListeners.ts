'use client';

import { useEffect } from 'react';
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
 * A `transcription-error` no longer stops anything here (specs/0083). It used to call the
 * page's post-stop handler with `callApi=false`, which only reset the UI's status while the
 * backend kept recording — a per-chunk engine failure leaves the recording running. The
 * user-facing toast/modal comes from useModalState's global listener; stopping is Rust's.
 */
export function useTranscriptionErrorListeners() {
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
    // Register once for the lifetime of the screen.
  }, []);
}
