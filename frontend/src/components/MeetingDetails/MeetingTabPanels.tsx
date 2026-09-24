'use client';

import dynamic from 'next/dynamic';
import { TranscriptPanel } from '@/components/MeetingDetails/TranscriptPanel';
import { SummaryPanel } from '@/components/MeetingDetails/SummaryPanel';
import { ActionItemsSection } from '@/components/MeetingDetails/ActionItemsSection';
import { PrepPanel } from '@/components/MeetingDetails/PrepPanel';
import { ModelConfig } from '@/components/ModelSettingsModal';
import type { TranscriptSegmentData } from '@/types';
import type { useMeetingData } from '@/hooks/meeting-details/useMeetingData';
import type { useSummaryGeneration } from '@/hooks/meeting-details/useSummaryGeneration';
import type { useTemplates } from '@/hooks/meeting-details/useTemplates';
import type { useCopyOperations } from '@/hooks/meeting-details/useCopyOperations';
import type { useMeetingOperations } from '@/hooks/meeting-details/useMeetingOperations';
import type { MeetingTabKey } from '@/hooks/meeting-details/useMeetingTabs';
import type { UseSpeakersReturn } from '@/hooks/useSpeakers';

// Client-only: NotepadPanel uses BlockNote (touches `document`), so keep it off the SSR path.
const NotepadPanel = dynamic(
  () => import('@/app/_components/NotepadPanel').then((m) => m.NotepadPanel),
  { ssr: false },
);

interface MeetingTabPanelsProps {
  meeting: {
    id: string;
    title: string;
    created_at: string;
    folder_path?: string | null;
    origin?: string | null;
  };
  isScheduled: boolean;
  isNotesOnly: boolean;
  activeTab: MeetingTabKey;
  /** Prep mounts lazily on first activation, then stays mounted (specs/0036). */
  hasOpenedPrep: boolean;
  meetingData: ReturnType<typeof useMeetingData>;
  summaryGeneration: ReturnType<typeof useSummaryGeneration>;
  templates: ReturnType<typeof useTemplates>;
  copyOperations: ReturnType<typeof useCopyOperations>;
  meetingOperations: ReturnType<typeof useMeetingOperations>;
  modelConfig: ModelConfig;
  setModelConfig: (config: ModelConfig | ((prev: ModelConfig) => ModelConfig)) => void;
  onSaveModelConfig: (config?: ModelConfig) => Promise<void>;
  /** Registers the summary toolbar's model-settings opener with the page. */
  onRegisterModalOpen: (openFn: () => void) => void;
  isRecording: boolean;
  // specs/0033 — search deep-link: scroll the Transcript tab to this segment.
  deepLinkSegmentId?: string | null;
  onDeepLinkConsumed?: () => void;
  // Pagination props for efficient transcript loading
  segments?: TranscriptSegmentData[];
  hasMore?: boolean;
  isLoadingMore?: boolean;
  totalCount?: number;
  loadedCount?: number;
  onLoadMore?: () => void;
  onRefetchTranscripts?: () => Promise<void>;
  /** The page-level speaker controller (specs/0057 Plan 3) — the channel strip lives
   *  above the tabs now, so the page owns it and the transcript's inline assignment
   *  shares it. */
  speakersController: UseSpeakersReturn;
  /** specs/0061 W4 (task 3) — a speaker clicked in the channel strip; filters the
   *  Transcript tab to just their lines. The PRIMARY key of the selected group (used
   *  for the chip's display-name lookup); see `speakerFilterKeys` for matching. */
  speakerFilter?: string | null;
  /** specs/0061 W4 task 3, ruling R36 — every member key of the selected consolidated
   *  group; TranscriptPanel matches a segment's `speaker` against any of these, not
   *  just the primary key. */
  speakerFilterKeys?: string[] | null;
  /** Clear the filter (the transcript's "Clear speaker filter" chip button). */
  onClearSpeakerFilter?: () => void;
}

/**
 * The meeting-details tab panels (Summary / Transcript / My notes / Prep). Panels are
 * kept mounted `display:none` so loaded state (transcript pages, notepad, prep brief)
 * persists across tab switches; which panels exist follows the meeting kind
 * (scheduled / notes-only / recorded).
 */
export function MeetingTabPanels({
  meeting,
  isScheduled,
  isNotesOnly,
  activeTab,
  hasOpenedPrep,
  meetingData,
  summaryGeneration,
  templates,
  copyOperations,
  meetingOperations,
  modelConfig,
  setModelConfig,
  onSaveModelConfig,
  onRegisterModalOpen,
  isRecording,
  deepLinkSegmentId,
  onDeepLinkConsumed,
  segments,
  hasMore,
  isLoadingMore,
  totalCount,
  loadedCount,
  onLoadMore,
  onRefetchTranscripts,
  speakersController,
  speakerFilter,
  speakerFilterKeys,
  onClearSpeakerFilter,
}: MeetingTabPanelsProps) {
  return (
    <>
      {/* SUMMARY — reuses the real generate/edit/regenerate path + all controls/states.
          Omitted for scheduled meetings (no recording yet — specs/0036). */}
      {!isScheduled && (
      <div
        role="tabpanel"
        id="meeting-tabpanel-summary"
        aria-labelledby="meeting-tab-summary"
        hidden={activeTab !== 'summary'}
        className={activeTab === 'summary' ? '' : 'hidden'}
      >
        <SummaryPanel
          variant="doc"
          meeting={meeting}
          meetingTitle={meetingData.meetingTitle}
          onTitleChange={meetingData.handleTitleChange}
          isEditingTitle={meetingData.isEditingTitle}
          onStartEditTitle={() => meetingData.setIsEditingTitle(true)}
          onFinishEditTitle={() => meetingData.setIsEditingTitle(false)}
          isTitleDirty={meetingData.isTitleDirty}
          summaryRef={meetingData.blockNoteSummaryRef}
          isSaving={meetingData.isSaving}
          onSaveAll={meetingData.saveAllChanges}
          onCopySummary={copyOperations.handleCopySummary}
          aiSummary={meetingData.aiSummary}
          summaryStatus={summaryGeneration.summaryStatus}
          transcripts={meetingData.transcripts}
          modelConfig={modelConfig}
          setModelConfig={setModelConfig}
          onSaveModelConfig={onSaveModelConfig}
          onGenerateSummary={summaryGeneration.handleGenerateSummary}
          onStopGeneration={summaryGeneration.handleStopGeneration}
          onSaveSummary={meetingData.handleSaveSummary}
          onSummaryChange={meetingData.handleSummaryChange}
          onDirtyChange={meetingData.setIsSummaryDirty}
          summaryError={summaryGeneration.summaryError}
          onRegenerateSummary={summaryGeneration.handleRegenerateSummary}
          availableTemplates={templates.availableTemplates}
          selectedTemplate={templates.selectedTemplate}
          onTemplateSelect={templates.handleTemplateSelection}
          isModelConfigLoading={false}
          onOpenModelSettings={onRegisterModalOpen}
        />

        {/* Action items (specs/0034) — derived from the summary, so they live below
            it. Refreshes on the `action-items-updated` event after extraction. */}
        <div className="mt-6">
          <ActionItemsSection
            meetingId={meeting.id}
            hasSummary={!!meetingData.aiSummary}
          />
        </div>
      </div>
      )}

      {/* TRANSCRIPT — existing list + pagination. Kept mounted so loaded pages persist.
          Omitted for notes-only and scheduled meetings (no recording/audio). */}
      {!isNotesOnly && !isScheduled && (
      <div
        role="tabpanel"
        id="meeting-tabpanel-transcript"
        aria-labelledby="meeting-tab-transcript"
        hidden={activeTab !== 'transcript'}
        className={activeTab === 'transcript' ? 'flex min-h-[40vh] flex-col' : 'hidden'}
      >
        <TranscriptPanel
          className="flex w-full min-h-[40vh] min-w-0 flex-col"
          transcripts={meetingData.transcripts}
          onCopyTranscript={copyOperations.handleCopyTranscript}
          onOpenMeetingFolder={meetingOperations.handleOpenMeetingFolder}
          isRecording={isRecording}
          disableAutoScroll={true}
          // specs/0033 — search deep-link: scroll to the matched segment (best-effort).
          // The scroll only runs while the Transcript tab is visible; on success
          // (or bounded give-up) the intent is consumed and the URL cleaned.
          scrollToSegmentId={deepLinkSegmentId ?? undefined}
          isScrollTargetVisible={activeTab === 'transcript'}
          onScrollToSegmentDone={onDeepLinkConsumed}
          // Pagination props for efficient loading
          usePagination={true}
          segments={segments}
          hasMore={hasMore}
          isLoadingMore={isLoadingMore}
          totalCount={totalCount}
          loadedCount={loadedCount}
          onLoadMore={onLoadMore}
          // Retranscription props
          meetingId={meeting.id}
          meetingFolderPath={meeting.folder_path}
          meetingOrigin={meeting.origin}
          onRefetchTranscripts={onRefetchTranscripts}
          speakersController={speakersController}
          speakerFilter={speakerFilter}
          speakerFilterKeys={speakerFilterKeys}
          onClearSpeakerFilter={onClearSpeakerFilter}
        />
      </div>
      )}

      {/* MY NOTES — lightweight notepad (loads + debounced-autosaves to meeting_notes). */}
      <div
        role="tabpanel"
        id="meeting-tabpanel-notes"
        aria-labelledby="meeting-tab-notes"
        hidden={activeTab !== 'notes'}
        className={activeTab === 'notes' ? 'min-h-[40vh]' : 'hidden'}
      >
        <NotepadPanel meetingId={meeting.id} showHeader={false} variant="bare" />
      </div>

      {/* PREP (specs/0036) — pre-call brief + carried-over open items + prep-notes
          agenda. The default (and, for a scheduled meeting, only content) tab for an
          upcoming occurrence; kept available on recorded meetings as pre-meeting
          context. Mounted lazily on first activation so opening a recorded meeting
          on another tab never fires prep generation the user didn't ask for. */}
      {(activeTab === 'prep' || hasOpenedPrep) && (
        <div
          role="tabpanel"
          id="meeting-tabpanel-prep"
          aria-labelledby="meeting-tab-prep"
          hidden={activeTab !== 'prep'}
          className={activeTab === 'prep' ? 'min-h-[40vh] pt-1' : 'hidden'}
        >
          {/* key by meeting id so navigating between meetings remounts with fresh brief
              state — otherwise a latched brief from the previous meeting would persist
              (code-review A1). */}
          <PrepPanel key={meeting.id} meetingId={meeting.id} />
        </div>
      )}
    </>
  );
}
