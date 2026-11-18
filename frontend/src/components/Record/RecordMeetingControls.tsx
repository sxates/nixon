'use client';

/**
 * The live Meeting's per-meeting keys, at the far right of the record screen's top strip
 * (owner feedback, v0.18): Participants, then the summary prompt picker. Transcript puts
 * them in the transcript's status strip after Pause transcript; My Notes and Agenda in the
 * main column's top strip. The page mounts ONE instance, in whichever strip is on screen,
 * so there is never a second popover or a second roster fetch.
 *
 * Every key is drawn like Pause transcript (`TranscriptModeButton`): an `xs` outline key on
 * `bg-card`, so the strip reads as one row of hardware keys.
 */

import { useState } from 'react';
import { ParticipantsPopover } from '@/components/Participants/ParticipantsPopover';
import { PromptPicker } from '@/components/SummaryPrompts/PromptPicker';
import { CustomPromptFlow, type PromptsApi } from '@/components/SummaryPrompts/CustomPromptFlow';

/** Pause transcript's look, shared by every key in the strip. */
export const STRIP_KEY_CLASS = 'bg-card';

interface RecordMeetingControlsProps {
  /** The live meeting row's SQLite id (`activeRecordingMeetingId`); null before it exists. */
  meetingId: string | null;
  isRecordingActive: boolean;
  /** The per-meeting prompt slice of useSummaryPrompts (specs/0079). */
  promptsApi: PromptsApi;
}

export function RecordMeetingControls({ meetingId, isRecordingActive, promptsApi }: RecordMeetingControlsProps) {
  const [customFlowOpen, setCustomFlowOpen] = useState(false);
  if (!meetingId && !isRecordingActive) return null;
  return (
    <div className="flex min-w-0 flex-shrink items-center gap-2">
      {/* Participants for the in-progress meeting (specs/0017). Use the authoritative SQLite
          meeting id, NOT the fabricated `currentMeetingId` from TranscriptContext (a
          `meeting-<timestamp>` IndexedDB id): adds made against that orphan id are lost at
          stop (specs/0024 WS3.1). */}
      {meetingId && <ParticipantsPopover meetingId={meetingId} size="xs" className={STRIP_KEY_CLASS} />}
      {/* Per-meeting summary prompt (specs/0079): persisted against the recording's id; the
          eventual summary uses it. */}
      {isRecordingActive && (
        <>
          <PromptPicker
            prompts={promptsApi.prompts}
            promptState={promptsApi.state}
            onPromptSelect={(id) => {
              void promptsApi.selectPrompt(id);
            }}
            onCustomPrompt={() => setCustomFlowOpen(true)}
            onClearCustomPrompt={() => {
              void promptsApi.clearOneOff();
            }}
            title="Summary prompt — used when this Meeting is summarized"
            className={`${STRIP_KEY_CLASS} min-w-0 max-w-[160px]`}
          />
          <CustomPromptFlow open={customFlowOpen} onClose={() => setCustomFlowOpen(false)} promptsApi={promptsApi} />
        </>
      )}
    </div>
  );
}
