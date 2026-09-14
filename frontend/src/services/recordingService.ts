/**
 * Recording Service
 *
 * Handles all recording lifecycle Tauri backend calls and events.
 * Pure 1-to-1 wrapper - no error handling changes, exact same behavior as direct invoke/listen calls.
 */

import { invoke } from '@tauri-apps/api/core';
import { listen, UnlistenFn } from '@tauri-apps/api/event';
import { recordStopRecordingResult, type StopRecordingResult } from '@/lib/recording-stop';

export interface RecordingState {
  is_recording: boolean;
  is_paused: boolean;
  is_active: boolean;
  recording_duration: number | null;
  active_duration: number | null;
}

export interface RecordingStoppedPayload {
  message: string;
  folder_path?: string;
  meeting_name?: string;
  // specs/0037 — set when this stop ends a RESUMED session. `resumed` routes the save to
  // APPEND to the same meeting (not the guarded new-row save); `prior_audio_duration_seconds`
  // is the offset the appended transcripts must be shifted by so their timeline is continuous.
  resumed?: boolean;
  prior_audio_duration_seconds?: number;
}

/**
 * Recording Service
 * Singleton service for managing recording lifecycle operations
 */
export class RecordingService {
  /**
   * Check if recording is currently active
   * @returns Promise<boolean>
   */
  async isRecording(): Promise<boolean> {
    return invoke<boolean>('is_recording');
  }

  /**
   * Get comprehensive recording state (includes durations)
   * @returns Promise with full recording state
   */
  async getRecordingState(): Promise<RecordingState> {
    return invoke<RecordingState>('get_recording_state');
  }

  /**
   * Get current meeting name
   * @returns Promise<string | null>
   */
  async getRecordingMeetingName(): Promise<string | null> {
    return invoke<string | null>('get_recording_meeting_name');
  }

  /**
   * Start recording (no device configuration)
   * @returns Promise<void>
   */
  async startRecording(): Promise<void> {
    return invoke('start_recording');
  }

  /**
   * Start recording with device configuration and meeting name
   * @param micDeviceName - Microphone device name (null for default)
   * @param systemDeviceName - System audio device name (null for none)
   * @param meetingName - Meeting name/title
   * @param opts - Optional identity/resume context (specs/0037):
   *   - `meetingId`: the freshly-created (or existing, on resume) DB meeting id. Passed on
   *     BOTH normal and resume starts so the backend can write it into the recording's
   *     `metadata.json` at start — the anchor crash recovery needs to reattach later.
   *   - `resumeFolderPath`: present ONLY on resume; tells the backend to REUSE that folder
   *     and append to the existing meeting instead of minting a fresh recording.
   *   When neither is known the payload stays byte-identical to before (`{ meetingName }`).
   * @returns Promise<void>
   */
  async startRecordingWithDevices(
    micDeviceName: string | null,
    systemDeviceName: string | null,
    meetingName: string,
    opts?: { meetingId?: string; resumeFolderPath?: string | null }
  ): Promise<void> {
    // The command's Tauri arg keys are camelCase (no rename_all on the Rust side).
    // This call historically sent snake_case keys, so every arg — including the
    // meeting title — silently deserialized to None and the backend minted its own
    // date-stamped name (which the stop path then wrote over calendar-event titles).
    //
    // Devices are deliberately NOT passed: the omitted-devices path resolves the
    // preferred mic/system device from the backend's recording-preference store
    // (kept in sync by the settings UI) and falls back to system defaults when a
    // preferred device is gone. The explicit-devices branch hard-errors instead of
    // falling back, and has never actually been exercised in production.
    //
    // `meetingId` rides along whenever it's known (normal OR resume). `resumeFolderPath`
    // is spread in ONLY on the resume path (keyed off the presence of the property, so a
    // deliberate null still routes as resume). A normal start with no known id sends the
    // exact same single-key payload as before (specs/0037).
    const payload: Record<string, unknown> = { meetingName };
    if (opts?.meetingId) {
      payload.meetingId = opts.meetingId;
    }
    if (opts && 'resumeFolderPath' in opts) {
      payload.resumeFolderPath = opts.resumeFolderPath ?? null;
    }
    return invoke('start_recording_with_devices_and_meeting', payload);
  }

  /**
   * Stop recording and save to file.
   *
   * specs/0037 (review-2): the command RETURNS the stop payload
   * ({ folder_path, meeting_name, resumed, prior_audio_duration_seconds }) — the same
   * data as the `recording-stopped` event, but race-free (the event is emitted only
   * after the possibly-slow stop_and_save). The result is stashed in memory (see
   * lib/recording-stop) as the save path's PRIMARY source; an older backend resolving
   * with undefined records nothing and the event transport takes over.
   *
   * @param savePath - Path to save audio file
   * @returns The normalized stop payload, or null when the backend returned none
   */
  async stopRecording(savePath: string): Promise<StopRecordingResult | null> {
    const result = await invoke<unknown>('stop_recording', {
      args: { save_path: savePath }
    });
    return recordStopRecordingResult(result);
  }

  /**
   * Pause active recording
   * @returns Promise<void>
   */
  async pauseRecording(): Promise<void> {
    return invoke('pause_recording');
  }

  /**
   * Resume paused recording
   * @returns Promise<void>
   */
  async resumeRecording(): Promise<void> {
    return invoke('resume_recording');
  }

  // Event Listeners

  /**
   * Listen for recording-started event
   * @param callback - Function to call when recording starts
   * @returns Promise that resolves to unlisten function
   */
  async onRecordingStarted(callback: () => void): Promise<UnlistenFn> {
    return listen('recording-started', callback);
  }

  /**
   * Listen for recording-stopped event (with metadata)
   * @param callback - Function to call when recording stops
   * @returns Promise that resolves to unlisten function
   */
  async onRecordingStopped(callback: (payload: RecordingStoppedPayload) => void): Promise<UnlistenFn> {
    return listen<RecordingStoppedPayload>('recording-stopped', (event) => {
      callback(event.payload);
    });
  }

  /**
   * Listen for recording-paused event
   * @param callback - Function to call when recording is paused
   * @returns Promise that resolves to unlisten function
   */
  async onRecordingPaused(callback: () => void): Promise<UnlistenFn> {
    return listen('recording-paused', callback);
  }

  /**
   * Listen for recording-resumed event
   * @param callback - Function to call when recording resumes
   * @returns Promise that resolves to unlisten function
   */
  async onRecordingResumed(callback: () => void): Promise<UnlistenFn> {
    return listen('recording-resumed', callback);
  }

  /**
   * Listen for chunk-drop-warning event (audio buffer overflow)
   * @param callback - Function to call when chunks are dropped
   * @returns Promise that resolves to unlisten function
   */
  async onChunkDropWarning(callback: (warning: string) => void): Promise<UnlistenFn> {
    return listen<string>('chunk-drop-warning', (event) => {
      callback(event.payload);
    });
  }

  /**
   * Listen for speech-detected event (VAD)
   * @param callback - Function to call when speech is detected
   * @returns Promise that resolves to unlisten function
   */
  async onSpeechDetected(callback: () => void): Promise<UnlistenFn> {
    return listen('speech-detected', callback);
  }
}

// Export singleton instance
export const recordingService = new RecordingService();
