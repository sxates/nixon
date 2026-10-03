"use client";

import { Summary, SummaryChunkStatus, Transcript } from '@/types';
import { BlockNoteSummaryView, BlockNoteSummaryViewRef } from '@/components/AISummary/BlockNoteSummaryView';
import { EmptyStateSummary } from '@/components/EmptyStateSummary';
import { SummaryGenerating } from './SummaryGenerating';
import { ModelConfig } from '@/components/ModelSettingsModal';
import { SummaryToolbar } from './SummaryToolbar';
import { useEffect, useRef, useState, RefObject } from 'react';
import { toast } from 'sonner';
import { AlertTriangle, Loader2 } from 'lucide-react';
import { useDiarizationActive } from '@/hooks/useDiarizationActive';
import { Button } from '@/components/ui/button';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog';
import { shouldConfirmPromptChange } from '@/lib/summary-prompts';
import { CustomPromptFlow, type PromptsApi } from '@/components/SummaryPrompts/CustomPromptFlow';

interface SummaryPanelProps {
  meeting: {
    id: string;
    title: string;
    created_at: string;
  };
  meetingTitle: string;
  onTitleChange: (title: string) => void;
  isEditingTitle: boolean;
  onStartEditTitle: () => void;
  onFinishEditTitle: () => void;
  isTitleDirty: boolean;
  summaryRef: RefObject<BlockNoteSummaryViewRef>;
  isSaving: boolean;
  onSaveAll: () => Promise<void>;
  onCopySummary: () => Promise<void>;
  aiSummary: Summary | null;
  summaryStatus: 'idle' | 'processing' | 'summarizing' | 'regenerating' | 'speaker_refresh' | 'completed' | 'error';
  transcripts: Transcript[];
  modelConfig: ModelConfig;
  setModelConfig: (config: ModelConfig | ((prev: ModelConfig) => ModelConfig)) => void;
  onSaveModelConfig: (config?: ModelConfig) => Promise<void>;
  onGenerateSummary: () => Promise<void>;
  onStopGeneration: () => void;
  onSaveSummary: (summary: Summary | { markdown?: string; summary_json?: any[] }) => Promise<void>;
  onSummaryChange: (summary: Summary) => void;
  onDirtyChange: (isDirty: boolean) => void;
  summaryError: string | null;
  onRegenerateSummary: () => Promise<void>;
  /** Saved prompts + this meeting's prompt state (specs/0079). */
  promptsApi: PromptsApi;
  isModelConfigLoading?: boolean;
  onOpenModelSettings?: (openFn: () => void) => void;
  /**
   * Layout variant.
   * - `panel` (default): self-contained card pane (`flex-1`, own scroll) for the
   *   legacy two-pane layout.
   * - `doc`: renders inline for the single-column document layout — the parent
   *   column owns the scroll/width, so this drops the card chrome and inner scroller
   *   while keeping every control and state.
   */
  variant?: 'panel' | 'doc';
}

export function SummaryPanel({
  meeting,
  meetingTitle,
  onTitleChange: _onTitleChange,
  isEditingTitle: _isEditingTitle,
  onStartEditTitle: _onStartEditTitle,
  onFinishEditTitle: _onFinishEditTitle,
  isTitleDirty,
  summaryRef,
  isSaving,
  onSaveAll,
  onCopySummary,
  aiSummary,
  summaryStatus,
  transcripts,
  modelConfig,
  setModelConfig,
  onSaveModelConfig,
  onGenerateSummary,
  onStopGeneration,
  onSaveSummary,
  onSummaryChange,
  onDirtyChange,
  summaryError,
  onRegenerateSummary,
  promptsApi,
  isModelConfigLoading = false,
  onOpenModelSettings,
  variant = 'panel',
}: SummaryPanelProps) {
  const isDoc = variant === 'doc';
  const activeMeetingIdRef = useRef(meeting.id);
  activeMeetingIdRef.current = meeting.id;

  // Deliberately NOT 'speaker_refresh' (specs/0041 WS2): the quiet post-diarization
  // refresh keeps the existing summary on screen — only the status strip below the
  // summary announces "Updating with speaker names…"; no spinner takeover.
  const isSummaryLoading = summaryStatus === 'processing' || summaryStatus === 'summarizing' || summaryStatus === 'regenerating';
  // A summary written before the speakers were known: the backend regenerates it with names
  // once identification lands (`speaker_refresh` is that regeneration running).
  const diarizationActive = useDiarizationActive(meeting.id);
  const preliminary = diarizationActive || summaryStatus === 'speaker_refresh';

  // ── Confirm-before-regenerate (specs/0020 task 8, now keyed on prompt id) ───────
  // Picking a DIFFERENT prompt while a summary is displayed must ask before anything
  // persists (regeneration costs minutes on local models). Cancel leaves the selection
  // untouched. With no summary the selection persists silently.
  const { prompts, state: promptState, selectPrompt, clearOneOff } = promptsApi;
  const [pendingPrompt, setPendingPrompt] = useState<{ id: string; name: string } | null>(null);
  // Latched so the confirm dialog's copy doesn't flip while it animates closed.
  const lastPendingNameRef = useRef('');
  if (pendingPrompt) lastPendingNameRef.current = pendingPrompt.name;
  // After confirm, regeneration waits until the pick has persisted and the refreshed
  // state reports it.
  const [regenerateArmedId, setRegenerateArmedId] = useState<string | null>(null);
  const [customFlowOpen, setCustomFlowOpen] = useState(false);

  const hasExistingSummary = !!aiSummary && !isSummaryLoading;

  const dismissPending = () => setPendingPrompt(null);

  const handlePromptSelect = (promptId: string, promptName: string) => {
    // Any new pick invalidates a previously armed regeneration.
    setRegenerateArmedId(null);
    if (shouldConfirmPromptChange(promptId, promptState, hasExistingSummary)) {
      setPendingPrompt({ id: promptId, name: promptName });
      return;
    }
    void selectPrompt(promptId);
  };

  const handleConfirmRegenerate = async () => {
    if (!pendingPrompt) return;
    const { id } = pendingPrompt;
    setPendingPrompt(null);
    // Arm only when the pick actually persisted; a failed pick must not leave a
    // stale arm that a later successful pick would fire a second time.
    if (await selectPrompt(id)) setRegenerateArmedId(id);
  };

  useEffect(() => {
    if (!regenerateArmedId || promptState?.prompt_id !== regenerateArmedId) return;
    setRegenerateArmedId(null);
    void onRegenerateSummary();
  }, [regenerateArmedId, promptState?.prompt_id, onRegenerateSummary]);

  const toolbarPromptProps = {
    prompts,
    promptState,
    onPromptSelect: handlePromptSelect,
    onCustomPrompt: () => setCustomFlowOpen(true),
    onClearCustomPrompt: () => {
      void clearOneOff().then((cleared) => {
        if (cleared) toast.info('Regenerate to apply');
      });
    },
  };

  // Partial-summary indicator (specs/0028 `summary_status`, surfaced in 0030 WS4).
  // The backend attaches chunk-outcome accounting to the stored result: when some
  // transcript chunks failed after retries the summary is PARTIAL and must not be
  // presented as complete. Markdown-format summaries carry the field through both the
  // saved-summary load (meeting-details/page.tsx) and the generation hook; absent
  // (older results, legacy format) means no warning.
  const chunkStatus = (aiSummary as { summary_status?: SummaryChunkStatus } | null)?.summary_status;
  const partialSummary = chunkStatus && chunkStatus.complete === false ? chunkStatus : null;

  return (
    <div
      className={
        isDoc
          ? 'summary-doc flex min-w-0 flex-col'
          : 'flex-1 min-w-0 flex flex-col bg-card overflow-hidden'
      }
    >
      {/* Summary action bar (title now lives in the meeting identity header).
          Only rendered when a summary exists, so there's no empty bordered strip. */}
      {aiSummary && !isSummaryLoading && (
        <div className={isDoc ? 'pb-3' : 'p-4 border-b border-border'}>
          <div
            className={
              isDoc
                ? 'flex flex-wrap items-center gap-1.5'
                : 'flex items-center justify-center w-full pt-0 gap-2'
            }
          >
            <div className="flex-shrink-0">
              <SummaryToolbar
                modelConfig={modelConfig}
                setModelConfig={setModelConfig}
                onSaveModelConfig={onSaveModelConfig}
                onGenerateSummary={onGenerateSummary}
                onStopGeneration={onStopGeneration}
                summaryStatus={summaryStatus}
                {...toolbarPromptProps}
                hasTranscripts={transcripts.length > 0}
                hasSummary={!!aiSummary}
                isModelConfigLoading={isModelConfigLoading}
                onOpenModelSettings={onOpenModelSettings}
                isSaving={isSaving}
                isDirty={isTitleDirty || (summaryRef.current?.isDirty || false)}
                onSave={onSaveAll}
                onCopy={onCopySummary}
              />
            </div>
          </div>
        </div>
      )}

      {isSummaryLoading ? (
        <div className={isDoc ? 'flex flex-col min-h-[40vh]' : 'flex flex-col h-full'}>
          {/* Show button group during generation */}
          <div className={isDoc ? 'flex items-center justify-start pb-6' : 'flex items-center justify-center pt-8 pb-4'}>
            <SummaryToolbar
              modelConfig={modelConfig}
              setModelConfig={setModelConfig}
              onSaveModelConfig={onSaveModelConfig}
              onGenerateSummary={onGenerateSummary}
              onStopGeneration={onStopGeneration}
              summaryStatus={summaryStatus}
              {...toolbarPromptProps}
              hasTranscripts={transcripts.length > 0}
              hasSummary={!!aiSummary}
              isModelConfigLoading={isModelConfigLoading}
              onOpenModelSettings={onOpenModelSettings}
              isSaving={isSaving}
              isDirty={isTitleDirty || (summaryRef.current?.isDirty || false)}
              onSave={onSaveAll}
              onCopy={onCopySummary}
            />
          </div>
          <SummaryGenerating />
        </div>
      ) : !aiSummary ? (
        <div className={isDoc ? 'flex flex-col min-h-[40vh]' : 'flex flex-col h-full'}>
          {/* Centered Summary Generator Button Group when no summary */}
          <div className={isDoc ? 'flex items-center justify-start gap-2 pb-4' : 'flex items-center justify-center gap-2 pt-8 pb-4'}>
            <SummaryToolbar
              modelConfig={modelConfig}
              setModelConfig={setModelConfig}
              onSaveModelConfig={onSaveModelConfig}
              onGenerateSummary={onGenerateSummary}
              onStopGeneration={onStopGeneration}
              summaryStatus={summaryStatus}
              {...toolbarPromptProps}
              hasTranscripts={transcripts.length > 0}
              hasSummary={false}
              isModelConfigLoading={isModelConfigLoading}
              onOpenModelSettings={onOpenModelSettings}
              isSaving={isSaving}
              isDirty={isTitleDirty || (summaryRef.current?.isDirty || false)}
              onSave={onSaveAll}
              onCopy={onCopySummary}
            />
          </div>
          {/* Empty state message */}
          <EmptyStateSummary
            onGenerate={() => onGenerateSummary()}
            hasModel={modelConfig.provider !== null && modelConfig.model !== null}
            isGenerating={isSummaryLoading}
          />
        </div>
      ) : transcripts?.length > 0 && (
        <div className={isDoc ? 'w-full' : 'flex-1 overflow-y-auto min-h-0'}>
          <div className={isDoc ? 'w-full' : 'p-6 w-full'}>
            {/* Partial-summary warning (specs/0028): some transcript chunks failed
                after retries, so sections of this summary may be missing. */}
            {/* Status sits above the summary, not under pages of it. */}
            {summaryStatus === 'error' && (
              <div
                role="alert"
                className="mb-4 flex items-start gap-2 rounded-md border border-destructive/30 bg-destructive/10 px-3 py-2"
              >
                <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0 text-destructive" aria-hidden="true" />
                <p className="text-sm text-foreground">
                  <span className="font-semibold">Could not generate the summary</span>
                  {summaryError ? <> — {summaryError}</> : null}
                </p>
              </div>
            )}
            {preliminary && summaryStatus !== 'error' && (
              <div
                role="status"
                className="mb-4 flex items-center gap-2 rounded-md border border-border bg-muted/50 px-3 py-2"
              >
                <Loader2 className="h-4 w-4 shrink-0 animate-spin text-muted-foreground" aria-hidden="true" />
                <p className="text-sm text-muted-foreground">
                  <span className="font-semibold text-foreground">Preliminary summary</span> — speakers
                  will be added when available.
                </p>
              </div>
            )}
            {partialSummary && (
              <div
                role="status"
                className="mb-4 flex items-start gap-2 rounded-md border border-brand/30 bg-brand/10 px-3 py-2"
              >
                <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0 text-brand" aria-hidden="true" />
                <p className="text-sm text-foreground">
                  <span className="font-semibold">Partial summary</span> — some sections may be
                  missing: {partialSummary.failed_chunks} of {partialSummary.total_chunks} transcript
                  section{partialSummary.total_chunks === 1 ? '' : 's'} could not be processed.
                  Regenerating the summary may recover them.
                </p>
              </div>
            )}
            <BlockNoteSummaryView
              ref={summaryRef}
              summaryData={aiSummary}
              onSave={onSaveSummary}
              onSummaryChange={onSummaryChange}
              onDirtyChange={onDirtyChange}
              status={summaryStatus}
              error={summaryError}
              onRegenerateSummary={() => {
                onRegenerateSummary();
              }}
              meeting={{
                id: meeting.id,
                title: meetingTitle,
                created_at: meeting.created_at
              }}
            />
          </div>
        </div>
      )}

      {/* Confirm-before-regenerate (specs/0020 task 8). Controlled dialog, same
          pattern as DeleteMeetingDialog: Cancel discards the pending pick (nothing
          was persisted), the primary action persists it and regenerates. */}
      <Dialog
        open={!!pendingPrompt}
        onOpenChange={(next) => {
          if (!next) dismissPending();
        }}
      >
        <DialogContent className="max-w-md">
          <DialogHeader>
            <DialogTitle>Regenerate summary?</DialogTitle>
            <DialogDescription>
              Regenerate the summary with the prompt &ldquo;{lastPendingNameRef.current}&rdquo;? This replaces
              the current summary.
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="outline" onClick={dismissPending}>
              Cancel
            </Button>
            <Button variant="brand" onClick={() => void handleConfirmRegenerate()} disabled={!pendingPrompt}>
              Regenerate
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      {/* One-off prompt + "keep this?" offer. A new instruction on an existing summary
          must take effect, so finishing the flow regenerates (never without a summary). */}
      <CustomPromptFlow
        open={customFlowOpen}
        onClose={() => setCustomFlowOpen(false)}
        promptsApi={promptsApi}
        onFinished={() => {
          if (hasExistingSummary) void onRegenerateSummary();
        }}
      />

      {/* One instance for the whole panel: the toolbar renders in three different states
          (summary present, generating, none yet) and its "…" flyout can open the language
          picker from any of them. Mounting it per-branch left the generating state opening
          nothing at all. */}
    </div>
  );
}
