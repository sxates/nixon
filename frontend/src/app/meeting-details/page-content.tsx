"use client";
import { useCallback, useMemo, useRef, useState } from 'react';
import { motion } from 'framer-motion';
import { useRouter, useSearchParams } from 'next/navigation';
import { invoke } from '@tauri-apps/api/core';
import { Summary } from '@/types';
import { useSidebar } from '@/components/Sidebar/SidebarProvider';
import { MeetingIdentityHeader } from '@/components/MeetingDetails/MeetingIdentityHeader';
import { DeleteMeetingDialog } from '@/components/MeetingDetails/DeleteMeetingDialog';
import { ParticipantsPanel } from '@/components/Participants/ParticipantsPanel';
import { ScheduledRecordControl } from '@/components/MeetingDetails/ScheduledRecordControl';
import { MeetingOptionsMenu } from '@/components/MeetingDetails/MeetingOptionsMenu';
import { MeetingTabsBar } from '@/components/MeetingDetails/MeetingTabsBar';
import { MeetingTabPanels } from '@/components/MeetingDetails/MeetingTabPanels';
import { SpeakerLegend } from '@/components/MeetingDetails/SpeakerLegend';

// Custom hooks
import { useMeetingData } from '@/hooks/meeting-details/useMeetingData';
import { useSummaryGeneration } from '@/hooks/meeting-details/useSummaryGeneration';
import { useTemplates } from '@/hooks/meeting-details/useTemplates';
import { useCopyOperations } from '@/hooks/meeting-details/useCopyOperations';
import { useMeetingOperations } from '@/hooks/meeting-details/useMeetingOperations';
import { useMeetingTabs, MeetingTabKey } from '@/hooks/meeting-details/useMeetingTabs';
import { useAutoGenerateSummary } from '@/hooks/meeting-details/useAutoGenerateSummary';
import { useModelSettings } from '@/hooks/meeting-details/useModelSettings';
import { useSpeakers } from '@/hooks/useSpeakers';
import { consolidateSpeakers } from '@/lib/speaker-consolidation';
import { useConfig } from '@/contexts/ConfigContext';
import { useRecordingState } from '@/contexts/RecordingStateContext';
import { armResumeRecording } from '@/lib/resume-recording';

/** specs/0057 — how the recording got here, for the reel label's SOURCE line.
 *  Only what the backend actually knows; an unmapped origin leaves the line blank. */
const REEL_SOURCE_BY_ORIGIN: Record<string, string> = {
  recorded: 'Recorded',
  imported: 'Imported',
  notes_only: 'Notes only',
};

export default function PageContent({
  meeting,
  summaryData,
  shouldAutoGenerate = false,
  onAutoGenerateComplete,
  onMeetingUpdated,
  onRefetchTranscripts,
  // specs/0033 — search deep-link: open the Transcript tab and scroll to this segment.
  deepLinkSegmentId,
  onDeepLinkConsumed,
  // specs/0061 W4 (task 3) — seed a scroll intent from code (a clicked speaker's
  // first line), reusing the same deep-link machinery as the search hit above.
  requestSegmentScroll,
  // Pagination props for efficient transcript loading
  segments,
  hasMore,
  isLoadingMore,
  totalCount,
  loadedCount,
  onLoadMore,
}: {
  meeting: any;
  summaryData: Summary | null;
  shouldAutoGenerate?: boolean;
  onAutoGenerateComplete?: () => void;
  onMeetingUpdated?: () => Promise<void>;
  onRefetchTranscripts?: () => Promise<void>;
  deepLinkSegmentId?: string | null;
  /** specs/0033 — consume the deep-link intent (scroll done or impossible). */
  onDeepLinkConsumed?: () => void;
  /** specs/0061 W4 (task 3) — `useSegmentDeepLink().request`, exposed by page.tsx. */
  requestSegmentScroll?: (id: string) => void;
  // Pagination props
  segments?: any[];
  hasMore?: boolean;
  isLoadingMore?: boolean;
  totalCount?: number;
  loadedCount?: number;
  onLoadMore?: () => void;
}) {
  console.log('📄 PAGE CONTENT: Initializing with data:', {
    meetingId: meeting.id,
    summaryDataKeys: summaryData ? Object.keys(summaryData) : null,
    transcriptsCount: meeting.transcripts?.length
  });

  const router = useRouter();
  const searchParams = useSearchParams();

  // State
  const [customPrompt, setCustomPrompt] = useState<string>('');
  // Notes-only meetings (spec 0015) have no recording/transcript/audio: drop the
  // Transcript tab and default to My notes. Recorded/imported meetings are unchanged.
  const isNotesOnly = meeting.origin === 'notes_only';
  // Scheduled meetings (specs/0036) are upcoming occurrences with no recording yet:
  // show ONLY Prep + My notes and default to Prep. `origin` is a free-form string on
  // the backend, so compare loosely (the TS DTO union doesn't include 'scheduled').
  const isScheduled = meeting.origin === 'scheduled';
  // The Today view deep-links to `?tab=prep` to open the Prep tab directly.
  const wantsPrepTab = searchParams.get('tab') === 'prep';
  // specs/0060 — the screenshot pipeline deep-links `?tab=summary|transcript|notes|prep`
  // straight to a tab; an unrecognized value is ignored by the hook.
  const requestedTab = searchParams.get('tab') as MeetingTabKey | null;

  // Tab state: which tabs exist, the active tab (deep-link/scheduled/notes-only rules),
  // lazy Prep mounting, and roving-focus keyboard handling (specs/0033, 0036, 0028).
  const { activeTab, setActiveTab, hasOpenedPrep, tabs, tabRefs, handleTabKeyDown } =
    useMeetingTabs({ isScheduled, isNotesOnly, wantsPrepTab, deepLinkSegmentId, onDeepLinkConsumed, requestedTab });

  // specs/0061 W4 (task 3) — click a speaker in the channel strip to filter the
  // transcript to just their lines and jump to the first one. Owned here (not the
  // legend or the panel) because it drives BOTH: the legend's row highlight and
  // the transcript tab's filter + tab switch + scroll. The lookup/filter logic
  // itself lives further down, once `speakersController` (needed to resolve a
  // consolidated group's member keys — ruling R36) is available.
  const [speakerFilter, setSpeakerFilter] = useState<string | null>(null);

  // Model-settings modal registration + save-config IPC.
  const { handleRegisterModalOpen, handleOpenModelSettings, handleSaveModelConfig } =
    useModelSettings();

  // Sidebar context
  const { refetchMeetings, activeRecordingMeetingId } = useSidebar();

  // Live recording state — used to hide "Continue recording" while any recording is active.
  const { isRecording: isLiveRecording } = useRecordingState();

  // Is THIS meeting the one being recorded right now? (Previously a hardcoded `false`
  // inherited from the old layout, so the recording gates below never fired.) A live
  // recording of some OTHER meeting must not blank this page's channel strip.
  const isRecording = isLiveRecording && meeting.id === activeRecordingMeetingId;

  // Delete-meeting confirm dialog (spec 0015 Phase A).
  const [isDeleteDialogOpen, setIsDeleteDialogOpen] = useState(false);

  const handleMeetingDeleted = async () => {
    await refetchMeetings();
    router.push('/');
  };

  // Continue recording (specs/0037): resume capture INTO this same meeting (same id +
  // folder), not a separate row. Only offered for a recorded meeting that actually has a
  // recording (folder_path), and never while a recording is already live. `origin` is
  // absent on pre-0015 rows and means 'recorded'.
  const isRecorded = meeting.origin === 'recorded' || meeting.origin == null;
  const canContinueRecording = isRecorded && !!meeting.folder_path && !isLiveRecording;

  const handleContinueRecording = () => {
    armResumeRecording({
      meetingId: meeting.id,
      folderPath: meeting.folder_path ?? null,
      meetingName: meetingData.meetingTitle,
    });
    router.push('/record');
  };

  // Get model config from ConfigContext
  const { modelConfig, setModelConfig } = useConfig();

  // specs/0019 WS2.1/WS2.4 + specs/0057 Plan 3 — ONE shared speaker controller for the
  // whole meeting document. The channel strip is meeting identity (it sits above the
  // tabs, visible on every tab), and the transcript's inline assignment mutates the same
  // source of truth, so renaming from a transcript line refreshes the strip and vice versa.
  const speakersController = useSpeakers({
    meetingId: meeting.id,
    onMutated: onRefetchTranscripts,
  });

  // Consolidated speaker groups (specs/0019 WS2.4) — two raw diarization keys assigned
  // to the same Person collapse into one channel. Shared by the VOICES count below and
  // by the speaker-filter group resolution (ruling R36).
  const speakerGroups = useMemo(
    () => consolidateSpeakers(speakersController.speakers),
    [speakersController.speakers],
  );

  // VOICES on the reel label must match the channels the strip actually draws.
  // `|| undefined` keeps the row blank (not "0") before diarization.
  const voices = useMemo(() => speakerGroups.length || undefined, [speakerGroups]);

  // specs/0061 W4 (task 3), controller ruling R36 — a channel-strip row represents a
  // CONSOLIDATED group, identified by its primary key, but the group can span several
  // raw diarization keys (two speakers assigned to the same Person). Both the transcript
  // filter and the first-line lookup must cover every member key, not just the primary
  // one — the group's true earliest line, or a mid-conversation line, can belong to a
  // non-primary key. `api_first_segment_for_speaker` was widened (Task 1's Rust side,
  // this task's Rust side) to take the whole key list and order by `audio_start_time`
  // in SQL, since the id alone carries no timestamp for a frontend-side comparison.
  const resolveMemberKeys = useCallback(
    (primaryKey: string): string[] =>
      speakerGroups.find((g) => g.primary.speakerKey === primaryKey)?.keys ?? [primaryKey],
    [speakerGroups],
  );

  // The current filter's full member-key set, kept in sync with `speakerFilter` (and
  // with `speakersController.speakers`, e.g. after a merge/rename) — passed to
  // TranscriptPanel so it can match ANY member key, not just the primary.
  const speakerFilterKeys = useMemo(
    () => (speakerFilter ? resolveMemberKeys(speakerFilter) : null),
    [speakerFilter, resolveMemberKeys],
  );

  // A monotonic token guards against a stale first-segment lookup (ruling R39): if the
  // user clicks A then quickly clicks A again (clear) or B (a different speaker) before
  // A's `invoke` resolves, A's answer must not seed a scroll for a filter that's no
  // longer current.
  const selectionTokenRef = useRef(0);

  const onSelectSpeaker = useCallback(
    (key: string | null) => {
      if (!key || key === speakerFilter) {
        // Re-clicking the already-selected row (or an explicit clear) clears it, and
        // invalidates any lookup still in flight for the row just cleared.
        selectionTokenRef.current += 1;
        setSpeakerFilter(null);
        return;
      }
      const token = ++selectionTokenRef.current;
      setSpeakerFilter(key);
      setActiveTab('transcript');
      const memberKeys = resolveMemberKeys(key);
      void (async () => {
        try {
          const segmentId = await invoke<string | null>('api_first_segment_for_speaker', {
            meetingId: meeting.id,
            speakerKeys: memberKeys,
          });
          // Drop a stale resolution: a newer click (same or a different speaker)
          // already moved the token on.
          if (segmentId && selectionTokenRef.current === token) {
            requestSegmentScroll?.(segmentId);
          }
        } catch (error) {
          // Best-effort jump — the filter itself already applied above.
          console.error("Failed to locate the speaker's first line:", error);
        }
      })();
    },
    [speakerFilter, meeting.id, setActiveTab, requestSegmentScroll, resolveMemberKeys],
  );

  const onClearSpeakerFilter = useCallback(() => {
    selectionTokenRef.current += 1;
    setSpeakerFilter(null);
  }, []);

  // Custom hooks
  const meetingData = useMeetingData({ meeting, summaryData, onMeetingUpdated });
  // No explicit id (falls back to the sidebar's viewed meeting); the title feeds
  // the auto-select-by-title suggestion (specs/0020 task 10).
  const templates = useTemplates(undefined, meetingData.meetingTitle);

  const summaryGeneration = useSummaryGeneration({
    meeting,
    transcripts: meetingData.transcripts,
    modelConfig: modelConfig,
    isModelConfigLoading: false, // ConfigContext loads on mount
    selectedTemplate: templates.selectedTemplate,
    onMeetingUpdated,
    updateMeetingTitle: meetingData.updateMeetingTitle,
    setAiSummary: meetingData.setAiSummary,
    onOpenModelSettings: handleOpenModelSettings,
    // specs/0029 WS7.2: after a deferred (transcribe-before-summarize)
    // transcription lands new rows, refresh the transcript panel.
    refetchTranscripts: onRefetchTranscripts,
  });

  const copyOperations = useCopyOperations({
    meeting,
    transcripts: meetingData.transcripts,
    meetingTitle: meetingData.meetingTitle,
    aiSummary: meetingData.aiSummary,
    blockNoteSummaryRef: meetingData.blockNoteSummaryRef,
  });

  const meetingOperations = useMeetingOperations({
    meeting,
  });

  // Auto-generate summary when the flag is set (specs/0029 WS7.2/WS7.3).
  useAutoGenerateSummary({
    shouldAutoGenerate,
    meetingId: meeting.id,
    folderPath: meeting.folder_path,
    transcriptCount: meetingData.transcripts.length,
    modelConfig,
    generateSummary: summaryGeneration.handleGenerateSummary,
    onAutoGenerateComplete,
  });

  return (
    <motion.div
      initial={{ opacity: 0, y: 20 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ duration: 0.3, ease: 'easeOut' }}
      className="flex h-page flex-col bg-background"
    >
      {/* Single-column document. The column owns the scroll; tab content renders inline.
          No top chrome bar — the back button lives inline to the left of the title. */}
      <div className="flex-1 overflow-y-auto">
        {/* Fluid reading column: full width of the content area up to a max cap (the
            content area already excludes the sidebar, so this never overflows). */}
        <div className="mx-auto w-full max-w-[840px] px-6 pb-24 pt-6 sm:px-7">
          {/* Compressed identity: inline back button + editable serif title + meta row.
              An unobtrusive overflow menu (delete) aligns to the right of the row. */}
          <div className="flex items-start gap-2">
            <div className="min-w-0 flex-1">
              <MeetingIdentityHeader
                onBack={() => {
                  // Return to wherever the user came from (day view, All meetings,
                  // People, sidebar) — every entry point `push`es, so history holds it
                  // (specs/0038 feedback #2). Fall back to Home when there's nothing to
                  // go back to (a cold deep-link) or when we arrived straight from a
                  // just-finished recording (back would land on the idle /record screen).
                  if (searchParams.get('source') === 'recording' || window.history.length <= 1) {
                    router.push('/');
                  } else {
                    router.back();
                  }
                }}
                meetingId={meeting.id}
                title={meetingData.meetingTitle}
                onTitleChange={meetingData.handleTitleChange}
                onSaveTitle={meetingData.handleSaveMeetingTitle}
                createdAt={meeting.created_at}
                transcripts={meetingData.transcripts}
                reelNumber={meeting.reelNumber}
                source={REEL_SOURCE_BY_ORIGIN[meeting.origin as string] ?? null}
                voices={voices}
              />
            </div>
            {/* Calendar-linked meeting: a compact countdown chip + record button bound to
                this event (specs/0038 feedback, reworked specs/0041 WS3). For a scheduled
                occurrence it's "Join & record" / "Start & record" (opens the call when the
                event has a link); for an already-recorded occurrence (false start) it
                becomes "Continue recording" via the specs/0037 resume path — same meeting
                row, never a duplicate. Sits inline next to the "…" menu. */}
            {meeting.calendarEventId && (isScheduled || isRecorded) && (
              <ScheduledRecordControl
                meetingId={meeting.id}
                startsAt={meeting.created_at}
                title={meetingData.meetingTitle}
                calendarEventId={meeting.calendarEventId}
                seriesKey={meeting.calendarSeriesKey ?? null}
                origin={isScheduled ? 'scheduled' : 'recorded'}
                hasTranscripts={(totalCount ?? meeting.transcripts?.length ?? 0) > 0}
                folderPath={meeting.folder_path ?? null}
              />
            )}
            <MeetingOptionsMenu
              canContinueRecording={canContinueRecording}
              onContinueRecording={handleContinueRecording}
              onDelete={() => setIsDeleteDialogOpen(true)}
            />
          </div>

          <DeleteMeetingDialog
            open={isDeleteDialogOpen}
            onOpenChange={setIsDeleteDialogOpen}
            meetingId={meeting.id}
            meetingTitle={meetingData.meetingTitle}
            onDeleted={handleMeetingDeleted}
          />

          {/* Participant roster (specs/0017) — the invited/known people for this
              meeting, seeded from the calendar event and hand-curatable. Distinct
              from the speaker legend (who actually spoke). Shown for every meeting
              including notes-only. */}
          <div className="mt-4">
            <ParticipantsPanel meetingId={meeting.id} />
          </div>

          {/* Channel strip (specs/0057 Plan 3) — who is on this reel, stated once on the
              document above the tabs rather than buried in the Transcript tab. Renders
              itself away for a meeting with no speakers (not diarized / notes-only). */}
          <SpeakerLegend
            meetingId={meeting.id}
            controller={speakersController}
            onRefetchTranscripts={onRefetchTranscripts}
            transcripts={meetingData.transcripts}
            isRecording={isRecording}
            selectedSpeakerKey={speakerFilter}
            onSelectSpeaker={onSelectSpeaker}
            className="mb-4"
          />

          {/* Tabs — underlined active tab in brand, sticky to the top of the scroll
              column (specs/0019 WS1.1). */}
          <MeetingTabsBar
            tabs={tabs}
            activeTab={activeTab}
            onSelect={setActiveTab}
            tabRefs={tabRefs}
            onTabKeyDown={handleTabKeyDown}
          />

          <MeetingTabPanels
            meeting={meeting}
            isScheduled={isScheduled}
            isNotesOnly={isNotesOnly}
            activeTab={activeTab}
            hasOpenedPrep={hasOpenedPrep}
            meetingData={meetingData}
            summaryGeneration={summaryGeneration}
            templates={templates}
            copyOperations={copyOperations}
            meetingOperations={meetingOperations}
            modelConfig={modelConfig}
            setModelConfig={setModelConfig}
            onSaveModelConfig={handleSaveModelConfig}
            onRegisterModalOpen={handleRegisterModalOpen}
            customPrompt={customPrompt}
            onPromptChange={setCustomPrompt}
            isRecording={isRecording}
            deepLinkSegmentId={deepLinkSegmentId}
            onDeepLinkConsumed={onDeepLinkConsumed}
            segments={segments}
            hasMore={hasMore}
            isLoadingMore={isLoadingMore}
            totalCount={totalCount}
            loadedCount={loadedCount}
            onLoadMore={onLoadMore}
            onRefetchTranscripts={onRefetchTranscripts}
            speakersController={speakersController}
            speakerFilter={speakerFilter}
            speakerFilterKeys={speakerFilterKeys}
            onClearSpeakerFilter={onClearSpeakerFilter}
          />
        </div>
      </div>
    </motion.div>
  );
}
