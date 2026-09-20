"use client";

import { Summary, SummaryChunkStatus, Transcript } from '@/types';
import { BlockNoteSummaryView, BlockNoteSummaryViewRef } from '@/components/AISummary/BlockNoteSummaryView';
import { EmptyStateSummary } from '@/components/EmptyStateSummary';
import { SummaryGenerating } from './SummaryGenerating';
import { ModelConfig } from '@/components/ModelSettingsModal';
import { SummaryToolbar } from './SummaryToolbar';
import { useEffect, useRef, useState, RefObject } from 'react';
import { AlertTriangle } from 'lucide-react';
import { Button } from '@/components/ui/button';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog';
import { shouldConfirmTemplateChange } from '@/hooks/meeting-details/useTemplates';

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
  onGenerateSummary: (customPrompt: string) => Promise<void>;
  onStopGeneration: () => void;
  customPrompt: string;
  onSaveSummary: (summary: Summary | { markdown?: string; summary_json?: any[] }) => Promise<void>;
  onSummaryChange: (summary: Summary) => void;
  onDirtyChange: (isDirty: boolean) => void;
  summaryError: string | null;
  onRegenerateSummary: () => Promise<void>;
  getSummaryStatusMessage: (status: 'idle' | 'processing' | 'summarizing' | 'regenerating' | 'speaker_refresh' | 'completed' | 'error') => string;
  availableTemplates: Array<{ id: string, name: string, description: string }>;
  selectedTemplate: string;
  onTemplateSelect: (templateId: string, templateName: string) => void;
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
  customPrompt,
  onSaveSummary,
  onSummaryChange,
  onDirtyChange,
  summaryError,
  onRegenerateSummary,
  getSummaryStatusMessage,
  availableTemplates,
  selectedTemplate,
  onTemplateSelect,
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

  // ── Confirm-before-regenerate (specs/0020 task 8) ──────────────────────────
  // Picking a DIFFERENT template while a summary is displayed must ask before
  // anything persists (regeneration costs minutes on local models). Cancel leaves
  // the selection untouched — nothing was persisted yet, so the dropdown + label
  // keep showing the template of the summary on screen. With no summary the
  // selection persists silently, exactly as before.
  const [pendingTemplate, setPendingTemplate] = useState<{ id: string; name: string } | null>(null);
  // After confirm, regeneration is deferred one render so `onRegenerateSummary`
  // (rebuilt by useSummaryGeneration from the NEW selectedTemplate) can't fire
  // with a stale template closure.
  const [regenerateArmedId, setRegenerateArmedId] = useState<string | null>(null);

  const hasExistingSummary = !!aiSummary && !isSummaryLoading;

  const handleTemplateSelect = (templateId: string, templateName: string) => {
    if (shouldConfirmTemplateChange(templateId, selectedTemplate, hasExistingSummary)) {
      setPendingTemplate({ id: templateId, name: templateName });
      return;
    }
    onTemplateSelect(templateId, templateName);
  };

  const handleConfirmRegenerate = () => {
    if (!pendingTemplate) return;
    onTemplateSelect(pendingTemplate.id, pendingTemplate.name); // persists via useTemplates
    setRegenerateArmedId(pendingTemplate.id);
    setPendingTemplate(null);
  };

  useEffect(() => {
    // Fire only once the parent re-rendered with the newly selected template, so
    // the regenerate handler passes the right templateId to the backend.
    if (!regenerateArmedId || selectedTemplate !== regenerateArmedId) return;
    setRegenerateArmedId(null);
    void onRegenerateSummary();
  }, [regenerateArmedId, selectedTemplate, onRegenerateSummary]);

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
                customPrompt={customPrompt}
                summaryStatus={summaryStatus}
                availableTemplates={availableTemplates}
                selectedTemplate={selectedTemplate}
                onTemplateSelect={handleTemplateSelect}
                hasTranscripts={transcripts.length > 0}
                hasSummary={!!aiSummary}
                isModelConfigLoading={isModelConfigLoading}
                onOpenModelSettings={onOpenModelSettings}
                isSaving={isSaving}
                isDirty={isTitleDirty || (summaryRef.current?.isDirty || false)}
                onSave={onSaveAll}
                onCopy={onCopySummary}
                meetingId={meeting.id}
                onRegenerate={onRegenerateSummary}
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
              customPrompt={customPrompt}
              summaryStatus={summaryStatus}
              availableTemplates={availableTemplates}
              selectedTemplate={selectedTemplate}
              onTemplateSelect={handleTemplateSelect}
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
              customPrompt={customPrompt}
              summaryStatus={summaryStatus}
              availableTemplates={availableTemplates}
              selectedTemplate={selectedTemplate}
              onTemplateSelect={handleTemplateSelect}
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
            onGenerate={() => onGenerateSummary(customPrompt)}
            hasModel={modelConfig.provider !== null && modelConfig.model !== null}
            isGenerating={isSummaryLoading}
          />
        </div>
      ) : transcripts?.length > 0 && (
        <div className={isDoc ? 'w-full' : 'flex-1 overflow-y-auto min-h-0'}>
          <div className={isDoc ? 'w-full' : 'p-6 w-full'}>
            {/* Partial-summary warning (specs/0028): some transcript chunks failed
                after retries, so sections of this summary may be missing. */}
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
          {summaryStatus !== 'idle' && (
            <div className={`mt-4 p-4 rounded-lg ${summaryStatus === 'error' ? 'bg-destructive/10 text-destructive' :
              summaryStatus === 'completed' ? 'bg-[hsl(var(--success)/0.12)] text-[hsl(var(--success))]' :
                'bg-brand/10 text-brand'
              }`}>
              <p className="text-sm font-medium">{getSummaryStatusMessage(summaryStatus)}</p>
            </div>
          )}
        </div>
      )}

      {/* Confirm-before-regenerate (specs/0020 task 8). Controlled dialog, same
          pattern as DeleteMeetingDialog: Cancel discards the pending pick (nothing
          was persisted), the primary action persists it and regenerates. */}
      <Dialog
        open={!!pendingTemplate}
        onOpenChange={(next) => {
          if (!next) setPendingTemplate(null);
        }}
      >
        <DialogContent className="max-w-md">
          <DialogHeader>
            <DialogTitle>Regenerate summary?</DialogTitle>
            <DialogDescription>
              Regenerate the summary with &ldquo;{pendingTemplate?.name}&rdquo;? This replaces
              the current summary.
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="outline" onClick={() => setPendingTemplate(null)}>
              Cancel
            </Button>
            <Button variant="brand" onClick={handleConfirmRegenerate} disabled={!pendingTemplate}>
              Regenerate
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      {/* One instance for the whole panel: the toolbar renders in three different states
          (summary present, generating, none yet) and its "…" flyout can open the language
          picker from any of them. Mounting it per-branch left the generating state opening
          nothing at all. */}
    </div>
  );
}
