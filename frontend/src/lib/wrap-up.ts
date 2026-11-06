/**
 * specs/0083 W2b — the meeting wrap-up, as the webview sees it.
 *
 * Rust owns everything that happens after a stop: drain, save, the defer marker and backlog
 * handoff, audio processing and the summary. It reports each stage as one `meeting-wrap-up`
 * event whose payload is the whole state (`audio/wrap_up/mod.rs::WrapUpState`). The webview
 * only reflects it: status for the rail and sidebar, a toast, the IndexedDB recovery mark.
 * Nothing here saves, summarizes or navigates.
 */

import { invoke } from '@tauri-apps/api/core';
import { toast } from 'sonner';

export const WRAP_UP_EVENT = 'meeting-wrap-up';

export type WrapUpStage =
  | 'stopping'
  | 'draining'
  | 'saving'
  | 'saved'
  | 'discarded'
  | 'handoff'
  | 'diarizing'
  | 'summarizing'
  | 'summary-skipped'
  | 'done'
  | 'failed';

export interface WrapUpToast {
  title: string;
  description?: string;
}

/** `WrapUpState` (camelCase). `reason`, `message`, `toast` and `failedStage` belong to the
 * stage that set them and are cleared on the next one. */
export interface WrapUpState {
  id: number;
  meetingId: string | null;
  source: string;
  stage: WrapUpStage;
  title: string | null;
  transcriptionIncomplete: boolean;
  reason: string | null;
  message: string | null;
  toast: WrapUpToast | null;
  failedStage: WrapUpStage | null;
}

/** Stages during which the recording is still being stopped, drained or saved. */
export function isSavingStage(stage: WrapUpStage): boolean {
  return stage === 'stopping' || stage === 'draining' || stage === 'saving';
}

/**
 * The `RecordingStatus` a stage maps to (spec W2b), or `null` when the stage leaves the
 * status alone (the meeting is saved; the rest happens in the queue). `saved` maps to
 * `completed`; the context then settles it to `idle`.
 */
export function wrapUpStatus(
  stage: WrapUpStage,
): 'processing' | 'saving' | 'completed' | 'idle' | 'error' | null {
  switch (stage) {
    case 'stopping':
    case 'draining':
      return 'processing';
    case 'saving':
      return 'saving';
    case 'saved':
      return 'completed';
    case 'discarded':
      return 'idle';
    case 'failed':
      return 'error';
    default:
      return null;
  }
}

export const SAVED_TOAST_TITLE = 'Meeting saved — processing';

/** Where the saved toast's View goes. */
export function meetingHref(meetingId: string): string {
  return `/meeting-details?id=${encodeURIComponent(meetingId)}`;
}

/**
 * Whether to show the saved toast: only while the window is visible (a hidden window has no
 * one to tell), and not when the window already shows that meeting's page.
 */
export function shouldShowSavedToast(input: {
  visible: boolean;
  pathname: string;
  search: string;
  meetingId: string;
}): boolean {
  if (!input.visible) return false;
  const onMeetingPage = input.pathname.replace(/\/$/, '') === '/meeting-details';
  if (!onMeetingPage) return true;
  return new URLSearchParams(input.search).get('id') !== input.meetingId;
}

/** Text of whatever the IPC layer threw. */
function messageOf(error: unknown): string {
  if (error instanceof Error) return error.message;
  if (typeof error === 'string') return error;
  return '';
}

/**
 * Stop the recording. Rust takes it from here and reports through `meeting-wrap-up`; the
 * command returns as soon as the wrap-up is accepted. A stop with nothing recording is a
 * no-op (a double click, or a stop that already landed from the tray).
 */
export async function requestStop(): Promise<void> {
  try {
    await invoke('stop_recording');
  } catch (error) {
    const message = messageOf(error);
    if (message.includes('No recording in progress')) return;
    console.error('[wrap-up] stop_recording failed:', error);
    toast.error('Couldn’t stop the recording', {
      description: message || 'Please try again.',
    });
  }
}
