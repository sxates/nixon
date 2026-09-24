'use client';

import { useState } from 'react';
import { useSidebar } from '@/components/Sidebar/SidebarProvider';
import { useRecordingState, RecordingStatus } from '@/contexts/RecordingStateContext';
import { useTranscripts } from '@/contexts/TranscriptContext';
import { useConfig } from '@/contexts/ConfigContext';
import { StatusOverlays } from '@/app/_components/StatusOverlays';
import { SettingsModals } from '@/app/_components/SettingsModal';
import { TranscriptPanel } from '@/app/_components/TranscriptPanel';
import { useModalState } from '@/hooks/useModalState';
import { useRecordingStateSync } from '@/hooks/useRecordingStateSync';
import { useRecordingStart } from '@/hooks/useRecordingStart';
import { useRecordingStop } from '@/hooks/useRecordingStop';
import { useTranscriptionErrorListeners } from '@/hooks/useTranscriptionErrorListeners';
import { useRecordingTitleEdit, TITLE_PLACEHOLDER } from '@/hooks/useRecordingTitleEdit';
import { useGlobalBarStop } from '@/hooks/useGlobalBarStop';
import { useStartupRecovery } from '@/hooks/useStartupRecovery';
import { TranscriptRecovery } from '@/components/TranscriptRecovery';
import { useTemplates } from '@/hooks/meeting-details/useTemplates';
import { RecordingHeader } from '@/components/Record/RecordingHeader';
import { RecordRail } from '@/components/Record/RecordRail';

export default function Home() {
  // Local page state (not moved to contexts)
  const [isRecording, setIsRecordingState] = useState(false);

  // Use contexts for state management
  const { meetingTitle } = useTranscripts();
  const { transcriptModelConfig } = useConfig();
  const recordingState = useRecordingState();

  // Extract status from global state
  const { status, isStopping, isProcessing } = recordingState;

  // Hooks
  const { setIsMeetingActive, isContentInsetCollapsed: sidebarCollapsed, activeRecordingMeetingId } = useSidebar();
  const { modals, messages, showModal, hideModal } = useModalState(transcriptModelConfig);
  const { setIsRecordingDisabled } = useRecordingStateSync(isRecording, setIsRecordingState, setIsMeetingActive);
  useRecordingStart(isRecording, setIsRecordingState, showModal);

  // Get handleRecordingStop function and setIsStopping (state comes from global context)
  const { handleRecordingStop, setIsStopping } = useRecordingStop(
    setIsRecordingState,
    setIsRecordingDisabled
  );

  // Transcription event listeners (transcription-error / speech-detected).
  // Mounted at the page level so registration is continuous across the not-recording→recording
  // transition — a control that remounts on that transition used to leave a gap where an
  // error event could be dropped. See specs/0014.
  useTranscriptionErrorListeners({
    onRecordingStop: (callApi = true) => handleRecordingStop(callApi),
  });

  // Rename-while-recording (specs/0029 WS4.3) — inline click-to-edit title state.
  const titleEdit = useRecordingTitleEdit();

  // ── Per-meeting summary template (specs/0029 WS4.3, the specs/0020 slice) ──
  // Explicitly bound to the recording's SQLite id; selections made before the row
  // exists are queued inside the hook and flushed once the id arrives.
  // The title feeds the auto-select-by-title suggestion (specs/0020 task 10);
  // the '+ New Call' placeholder is a UI affordance, not a real title, so it's
  // withheld until the user (or Join & Record) names the meeting.
  const { availableTemplates, selectedTemplate, handleTemplateSelection } =
    useTemplates(activeRecordingMeetingId, meetingTitle === TITLE_PLACEHOLDER ? null : meetingTitle);

  // Stop triggered from the transport rail on another route (flag + window event).
  useGlobalBarStop({ setIsStopping, handleRecordingStop });

  // Startup recovery checks + the once-per-session recovery dialog.
  const {
    showRecoveryDialog,
    recoverableMeetings,
    handleRecovery,
    handleDialogClose,
    deleteRecoverableMeeting,
    loadMeetingTranscripts,
  } = useStartupRecovery();

  // Computed values using global status
  const isProcessingStop = status === RecordingStatus.PROCESSING_TRANSCRIPTS || isProcessing;

  // Header chrome state. The elapsed clock lives on the transport rail's tape counter now.
  const isRecordingActive = recordingState.isRecording;

  return (
    <div className="flex flex-col h-page bg-background">
      {/* All Modals supported*/}
      <SettingsModals
        modals={modals}
        messages={messages}
        onClose={hideModal}
      />

      {/* Recovery Dialog */}
      <TranscriptRecovery
        isOpen={showRecoveryDialog}
        onClose={handleDialogClose}
        recoverableMeetings={recoverableMeetings}
        onRecover={handleRecovery}
        onDelete={deleteRecoverableMeeting}
        onLoadPreview={loadMeetingTranscripts}
      />

      {/* ── Recording header: back · title · identity line · template · mode · VU + lamps ──
          The transport rail (app/layout.tsx) carries REC/HOLD/STOP, the reels and the counter. */}
      <RecordingHeader
        meetingTitle={meetingTitle}
        isRecordingActive={isRecordingActive}
        activeRecordingMeetingId={activeRecordingMeetingId}
        titleEdit={titleEdit}
        templates={{ availableTemplates, selectedTemplate, handleTemplateSelection }}
      />

      <div className="relative flex flex-1 overflow-hidden">
        {/* Split layout: live transcript (left) + Notes / Prep rail (right, ~40% / max 440px;
            specs/0056 W4). */}
        <div className="flex flex-1 min-w-0 overflow-hidden">
          <div className="min-w-0 flex flex-1 flex-col overflow-hidden">
            <TranscriptPanel
              isProcessingStop={isProcessingStop}
              isStopping={isStopping}
            />
          </div>
          <RecordRail meetingId={activeRecordingMeetingId ?? null} />
        </div>

        {/* Status Overlays - Processing and Saving */}
        <StatusOverlays
          isProcessing={status === RecordingStatus.PROCESSING_TRANSCRIPTS && !recordingState.isRecording}
          isSaving={status === RecordingStatus.SAVING}
          sidebarCollapsed={sidebarCollapsed}
        />
      </div>
    </div>
  );
}
