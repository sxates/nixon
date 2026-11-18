'use client';

/**
 * The meeting bar: the 44px strip at the top of the record screen's left / main column in
 * every view (owner feedback, v0.18). Left: the live Meeting's Topic chips (each opens its
 * Topic) and "Add Topic". Right: Participants, then the summary prompt picker. No label: the
 * header's view tab names what's below.
 *
 * The page mounts ONE instance, in whichever column is on screen (the transcript's top in
 * Transcript, the main column's top in My Notes / Agenda), so there is never a second
 * popover, roster fetch or Topics load.
 *
 * Before the meeting row exists the bar is an empty band rather than gone, so the left
 * column's top rule still meets the right column's first strip and nothing jumps when the
 * recording starts.
 *
 * Every key is drawn like Pause transcript (`TranscriptModeButton`): an `xs` outline key on
 * `bg-card`, so the bars read as rows of hardware keys.
 */

import { useState } from 'react';
import { ParticipantsPopover } from '@/components/Participants/ParticipantsPopover';
import { PromptPicker } from '@/components/SummaryPrompts/PromptPicker';
import { CustomPromptFlow, type PromptsApi } from '@/components/SummaryPrompts/CustomPromptFlow';
import { MeetingTopicsRow } from '@/components/Topics/MeetingTopicsRow';
import { RECORD_STRIP_HEIGHT } from '@/lib/record-view';
import { cn } from '@/lib/utils';

/** Pause transcript's look, shared by every key in the bars. */
export const STRIP_KEY_CLASS = 'bg-card';

interface RecordMeetingControlsProps {
  /** The live meeting row's SQLite id (`activeRecordingMeetingId`); null before it exists. */
  meetingId: string | null;
  /** The live title, for Add Topic's Series option. */
  meetingTitle: string | null;
  isRecordingActive: boolean;
  /** The per-meeting prompt slice of useSummaryPrompts (specs/0079). */
  promptsApi: PromptsApi;
}

export function RecordMeetingControls({
  meetingId,
  meetingTitle,
  isRecordingActive,
  promptsApi,
}: RecordMeetingControlsProps) {
  const [customFlowOpen, setCustomFlowOpen] = useState(false);
  return (
    <div
      data-testid="record-meeting-bar"
      className={cn(
        'flex flex-shrink-0 items-center gap-3 border-b border-border bg-card px-5 py-1.5',
        RECORD_STRIP_HEIGHT,
      )}
    >
      {/* Linking a Topic mid-recording needs nothing more: the Topic entry is written after
          the summary completes, when the summary job reads the Meeting's links. */}
      <div className="flex min-w-0 flex-1 items-center">
        {meetingId && (
          <MeetingTopicsRow
            meetingId={meetingId}
            title={meetingTitle}
            seriesKey={null}
            variant="strip"
            keyClassName={STRIP_KEY_CLASS}
            className=""
          />
        )}
      </div>
      <div className="flex min-w-0 flex-shrink-0 items-center gap-2">
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
              className={cn(STRIP_KEY_CLASS, 'min-w-0 max-w-[160px]')}
            />
            <CustomPromptFlow open={customFlowOpen} onClose={() => setCustomFlowOpen(false)} promptsApi={promptsApi} />
          </>
        )}
      </div>
    </div>
  );
}
