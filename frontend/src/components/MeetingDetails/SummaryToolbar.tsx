'use client';

import { useEffect, useState } from 'react';
import { Loader2, MoreHorizontal } from 'lucide-react';

import { ModelConfig, ModelSettingsModal } from '@/components/ModelSettingsModal';
import { Button } from '@/components/ui/button';
import { ButtonGroup } from '@/components/ui/button-group';
import {
  Dialog,
  DialogContent,
  DialogTitle,
} from '@/components/ui/dialog';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu';
import { VisuallyHidden } from '@/components/ui/visually-hidden';
import { PromptPicker } from '@/components/SummaryPrompts/PromptPicker';
import type { MeetingPromptState, SummaryPrompt } from '@/lib/summary-prompts';
import { useSummaryGenerationGuards } from '@/hooks/meeting-details/useSummaryGenerationGuards';

export type SummaryStatus =
  | 'idle'
  | 'processing'
  | 'summarizing'
  | 'regenerating'
  | 'speaker_refresh'
  | 'completed'
  | 'error';

export interface SummaryToolbarProps {
  /* Primary action */
  summaryStatus: SummaryStatus;
  hasSummary: boolean;
  hasTranscripts: boolean;
  isModelConfigLoading: boolean;
  onGenerateSummary: (customPrompt: string) => Promise<void>;
  onStopGeneration: () => void;

  /* Prompt */
  prompts: SummaryPrompt[];
  promptState: MeetingPromptState | null;
  onPromptSelect: (promptId: string, promptName: string) => void;
  /** Opens the "Custom for this meeting…" dialog (the panel owns it). */
  onCustomPrompt: () => void;
  onClearCustomPrompt: () => void;

  /* Flyout */
  isSaving: boolean;
  isDirty: boolean;
  onSave: () => Promise<void>;
  onCopy: () => Promise<void>;
  /** Opens the language picker, which the panel owns (it holds the language state). */
  modelConfig: ModelConfig;
  setModelConfig: (config: ModelConfig | ((prev: ModelConfig) => ModelConfig)) => void;
  onSaveModelConfig: (config?: ModelConfig) => Promise<void>;
  /** Lets the parent open model settings itself (the empty-state card links to it). */
  onOpenModelSettings?: (openFn: () => void) => void;
}

/**
 * The Summary tab's action bar (specs/0064 W3).
 *
 * Replaces the two button groups that between them showed up to eight controls — Stop,
 * Generate/Regenerate, the prompt picker,
 * Save and Copy — which the owner reported as "overkill". What is left is the primary action,
 * the prompt picker, and a "…" flyout for everything that is occasionally useful.
 *
 * Save is the exception that proves the rule: it lives in the flyout, but the instant the
 * summary or title has unsaved edits it appears as a real button, because that is the only
 * control here that is ever time-critical. Hunting for it inside a menu with unsaved work
 * pending is exactly the kind of thing a tidy toolbar must not cost you.
 *
 * The model dialog and the language picker are opened BY STATE from menu items rather than
 * being nested inside the menu's content: a Radix dialog/popover rendered inside menu content
 * unmounts along with the menu on the first outside click, which makes it impossible to use.
 */
export function SummaryToolbar({
  summaryStatus,
  hasSummary,
  hasTranscripts,
  isModelConfigLoading,
  onGenerateSummary,
  onStopGeneration,
  prompts,
  promptState,
  onPromptSelect,
  onCustomPrompt,
  onClearCustomPrompt,
  isSaving,
  isDirty,
  onSave,
  onCopy,
  modelConfig,
  setModelConfig,
  onSaveModelConfig,
  onOpenModelSettings,
}: SummaryToolbarProps) {
  const [settingsDialogOpen, setSettingsDialogOpen] = useState(false);

  const { isCheckingModels, generate } = useSummaryGenerationGuards({
    modelConfig,
      onGenerateSummary,
    onNeedsModelSettings: () => setSettingsDialogOpen(true),
  });

  // The empty-state card offers its own "open model settings" link; hand it the opener.
  useEffect(() => {
    onOpenModelSettings?.(() => setSettingsDialogOpen(true));
  }, [onOpenModelSettings]);

  // 'speaker_refresh' counts as generating here so the button flips to Stop (which cancels
  // the backend's quiet post-diarization refresh) instead of allowing a second, racing
  // generation to start (specs/0041 WS2).
  const isGenerating =
    summaryStatus === 'processing' ||
    summaryStatus === 'summarizing' ||
    summaryStatus === 'regenerating' ||
    summaryStatus === 'speaker_refresh';

  if (!hasTranscripts) {
    return null;
  }

  return (
    <>
      <ButtonGroup>
        {isGenerating ? (
          <Button
            variant="outline"
            size="xs"
            className="border-record/30 bg-record/10 text-record-ink hover:bg-record/20 xl:px-4"
            onClick={onStopGeneration}
            title="Stop summary generation"
          >
            <span>Stop</span>
          </Button>
        ) : (
          <Button
            variant="outline"
            size="xs"
            className="xl:px-4"
            onClick={() => {
              void generate();
            }}
            disabled={isCheckingModels || isModelConfigLoading}
            title={
              isModelConfigLoading
                ? 'Loading model configuration...'
                : isCheckingModels
                  ? 'Checking models...'
                  : hasSummary
                    ? 'Regenerate AI Summary'
                    : 'Generate AI Summary'
            }
          >
            {isCheckingModels || isModelConfigLoading ? (
              <>
                <Loader2 className="mr-2 animate-spin" size={16} />
                <span>Processing…</span>
              </>
            ) : (
              <span>{hasSummary ? 'Regenerate Summary' : 'Generate Summary'}</span>
            )}
          </Button>
        )}

        {/* Save is normally in the flyout; unsaved edits pull it out here. */}
        {isDirty && (
          <Button
            variant="outline"
            size="xs"
            className="bg-brand/20"
            title={isSaving ? 'Saving' : 'Save changes'}
            onClick={() => {
              void onSave();
            }}
            disabled={isSaving}
          >
            {isSaving ? (
              <>
                <Loader2 className="mr-2 animate-spin" size={16} />
                <span>Saving…</span>
              </>
            ) : (
              <span>Save</span>
            )}
          </Button>
        )}

        <PromptPicker
          prompts={prompts}
          promptState={promptState}
          onPromptSelect={onPromptSelect}
          onCustomPrompt={onCustomPrompt}
          onClearCustomPrompt={onClearCustomPrompt}
          className="max-w-[160px]"
        />

        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button
              variant="outline"
              size="xs"
              aria-label="More summary actions"
              title="More summary actions"
            >
              <MoreHorizontal size={14} />
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end">
            <DropdownMenuItem
              onSelect={() => {
                void onSave();
              }}
              disabled={isSaving}
            >
              Save
            </DropdownMenuItem>
            <DropdownMenuItem
              onSelect={() => {
                void onCopy();
              }}
              disabled={!hasSummary}
            >
              Copy
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
      </ButtonGroup>

      {/* Rendered outside the menu so it survives the menu closing. */}
      <Dialog open={settingsDialogOpen} onOpenChange={setSettingsDialogOpen}>
        <DialogContent aria-describedby={undefined}>
          <VisuallyHidden>
            <DialogTitle>Model Settings</DialogTitle>
          </VisuallyHidden>
          <ModelSettingsModal
            onSave={async (config) => {
              await onSaveModelConfig(config);
              setSettingsDialogOpen(false);
            }}
            modelConfig={modelConfig}
            setModelConfig={setModelConfig}
            skipInitialFetch={true}
            layout="dialog"
          />
        </DialogContent>
      </Dialog>
    </>
  );
}
