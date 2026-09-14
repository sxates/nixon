/**
 * Storage Service
 *
 * Handles all meeting storage and retrieval Tauri backend calls (SQLite persistence).
 * Pure 1-to-1 wrapper - no error handling changes, exact same behavior as direct invoke calls.
 */

import { invoke } from '@tauri-apps/api/core';
import { Transcript } from '@/types';

export interface SaveMeetingRequest {
  meetingTitle: string;
  transcripts: Transcript[];
  folderPath: string | null;
}

export interface SaveMeetingResponse {
  meeting_id: string;
}

export interface Meeting {
  id: string;
  title: string;
  [key: string]: any; // Allow additional properties from backend
}

/**
 * Storage Service
 * Singleton service for managing meeting storage operations
 */
export class StorageService {
  /**
   * Save meeting transcript to SQLite database.
   *
   * When `meetingId` is provided, the transcripts are attached to that EXISTING meeting row
   * (no new row is created) — this is the persist-at-start flow where the meeting was created
   * when recording began. Omitting `meetingId` keeps the legacy create-new-meeting behavior.
   *
   * When `resumed` is true (specs/0037), the backend APPENDS the transcripts to the given
   * `meetingId` instead of taking the guarded new-row save path, and shifts their timeline by
   * `audioOffsetSeconds` (the prior recording's duration) so the appended segments line up
   * after the earlier ones. Both default to false/0, leaving a normal save unchanged.
   *
   * @param meetingTitle - Title of the meeting
   * @param transcripts - Array of transcript segments
   * @param folderPath - Optional folder path for audio file
   * @param meetingId - Optional existing meeting id to attach transcripts to
   * @param resumed - Whether this save ends a resumed session (append vs. new-row save)
   * @param audioOffsetSeconds - Seconds to shift appended transcripts by (prior audio duration)
   * @returns Promise with { meeting_id: string }
   */
  async saveMeeting(
    meetingTitle: string,
    transcripts: Transcript[],
    folderPath: string | null,
    meetingId?: string | null,
    resumed?: boolean,
    audioOffsetSeconds?: number
  ): Promise<SaveMeetingResponse> {
    return invoke<SaveMeetingResponse>('api_save_transcript', {
      meetingTitle,
      transcripts,
      folderPath,
      meetingId: meetingId ?? null,
      resumed: resumed ?? false,
      audioOffsetSeconds: audioOffsetSeconds ?? 0,
    });
  }

  /**
   * Get meeting details by ID
   * @param meetingId - ID of the meeting to fetch
   * @returns Promise with meeting details
   */
  async getMeeting(meetingId: string): Promise<Meeting> {
    return invoke<Meeting>('api_get_meeting', { meetingId });
  }

  /**
   * Get list of all meetings
   * @returns Promise with array of meetings
   */
  async getMeetings(): Promise<Meeting[]> {
    return invoke<Meeting[]>('api_get_meetings');
  }
}

// Export singleton instance
export const storageService = new StorageService();
