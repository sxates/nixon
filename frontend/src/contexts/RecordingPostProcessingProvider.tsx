'use client';

import React, { useEffect } from 'react';
import { safeListen } from '@/lib/safe-listen';
import { useRecordingStop } from '@/hooks/useRecordingStop';

/**
 * RecordingPostProcessingProvider
 *
 * This provider handles post-processing when recording stops from any source:
 * - Tray menu stop
 * - Global keyboard shortcut
 * - Overlay stop button
 * - Main UI stop button
 *
 * It listens for the 'recording-stop-complete' event from Rust backend
 * and triggers the full post-processing flow (save to database, navigate)
 * regardless of which page the user is currently on.
 */
export function RecordingPostProcessingProvider({ children }: { children: React.ReactNode }) {
  // No-op functions since the global RecordingStateContext already handles state updates
  // These are only needed for the hook's local component state management
  const setIsRecording = () => { };
  const setIsRecordingDisabled = () => { };

  const {
    handleRecordingStop,
  } = useRecordingStop(setIsRecording, setIsRecordingDisabled);

  useEffect(() => {
    // Listen for recording-stop-complete event from Rust
    return safeListen<boolean>('recording-stop-complete', (event) => {
      console.log('[RecordingPostProcessing] Received recording-stop-complete event:', event.payload);

      // Call the post-processing handler
      // event.payload is the callApi boolean (true for normal stops)
      handleRecordingStop(event.payload);
    });
  }, [handleRecordingStop]);

  return <>{children}</>;
}
