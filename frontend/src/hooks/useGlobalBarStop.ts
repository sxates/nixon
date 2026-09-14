import { useEffect, useRef } from 'react';
import { appDataDir } from '@tauri-apps/api/path';
import { toast } from 'sonner';
import { useRecordingState } from '@/contexts/RecordingStateContext';
import { recordingService } from '@/services/recordingService';

interface UseGlobalBarStopParams {
  setIsStopping: (value: boolean) => void;
  handleRecordingStop: (callApi: boolean) => Promise<void> | void;
}

/**
 * Stop triggered from the transport rail (formerly GlobalRecordingBar) on another route.
 *
 * The bar can't run the full stop flow itself (the stop+save logic lives on /record, in
 * useRecordingStop, which is only mounted there). It navigates here and signals us via
 * BOTH a `stopRecordingOnLoad` sessionStorage flag (consumed on mount when the page wasn't
 * mounted yet — the common case) AND a live `stop-recording-from-global-bar` window event
 * (when /record is already mounted). We run the exact same two-part stop the on-page Stop
 * button does — the raw `stop_recording` command, then the post-stop processing/save via
 * handleRecordingStop(true) — so transcripts and folder_path persist correctly. A ref
 * guards against running it twice if both the flag and the event fire.
 */
export function useGlobalBarStop({ setIsStopping, handleRecordingStop }: UseGlobalBarStopParams): void {
  const recordingState = useRecordingState();
  const { isStopping } = recordingState;

  const globalStopInProgressRef = useRef(false);
  useEffect(() => {
    const runGlobalStop = async () => {
      sessionStorage.removeItem('stopRecordingOnLoad');
      if (globalStopInProgressRef.current) return;
      if (!recordingState.isRecording || isStopping) return;

      globalStopInProgressRef.current = true;
      console.log('[record] Stop requested from the transport rail');
      setIsStopping(true);
      try {
        const dataDir = await appDataDir();
        const timestamp = new Date().toISOString().replace(/[:.]/g, '-');
        const savePath = `${dataDir}/recording-${timestamp}.wav`;
        await recordingService.stopRecording(savePath);
        await handleRecordingStop(true);
      } catch (error) {
        console.error('[record] Failed to stop recording from global bar:', error);
        const msg = error instanceof Error ? error.message : String(error);
        // "No recording in progress" can happen if it was already stopped elsewhere — benign.
        if (!msg.includes('No recording in progress')) {
          // The retired RecordingControls.stopRecordingAction ran the post-stop flow with
          // callApi=false on any non-benign failure, so whatever was already captured still
          // got saved and processed locally. This is now the ONLY stop path — keep that
          // fallback, or a failed `stop_recording` silently loses the meeting.
          try {
            await handleRecordingStop(false);
          } catch (fallbackError) {
            console.error('[record] Local post-processing fallback also failed:', fallbackError);
          }
          toast.error('Failed to stop recording', { description: msg });
        }
        setIsStopping(false);
      } finally {
        globalStopInProgressRef.current = false;
      }
    };

    // Consume the on-load flag (set by the bar before navigating here).
    if (sessionStorage.getItem('stopRecordingOnLoad') === 'true') {
      void runGlobalStop();
    }

    window.addEventListener('stop-recording-from-global-bar', runGlobalStop);
    return () => {
      window.removeEventListener('stop-recording-from-global-bar', runGlobalStop);
    };
  }, [recordingState.isRecording, isStopping, setIsStopping, handleRecordingStop]);
}
