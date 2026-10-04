'use client';

import { useState } from 'react';
import { MAX_PROMPT_NAME_CHARS, type MeetingPromptState } from '@/lib/summary-prompts';
import type { useSummaryPrompts } from '@/hooks/meeting-details/useSummaryPrompts';
import { OneOffPromptDialog } from './OneOffPromptDialog';
import { SaveOneOffDialog } from './SaveOneOffDialog';

/** The slice of `useSummaryPrompts()` the per-meeting pickers use. */
export type PromptsApi = Pick<
  ReturnType<typeof useSummaryPrompts>,
  'prompts' | 'state' | 'selectPrompt' | 'saveOneOff' | 'clearOneOff' | 'saveFollowup'
>;

interface CustomPromptFlowProps {
  /** True from "Custom for this meeting…" until the whole flow is finished or cancelled. */
  open: boolean;
  onClose: () => void;
  promptsApi: Pick<PromptsApi, 'state' | 'saveOneOff' | 'saveFollowup'>;
  /** Runs once the follow-up offer is answered (Save or Not now), never on Cancel. */
  onFinished?: () => void;
}

interface FollowupSeed {
  hasSeries: boolean;
  defaultName: string;
}

function seedFrom(body: string, state: MeetingPromptState | null): FollowupSeed {
  const firstLine = body.trim().split('\n')[0] ?? '';
  return {
    hasSeries: state?.has_series ?? false,
    defaultName: firstLine.slice(0, MAX_PROMPT_NAME_CHARS).trim(),
  };
}

/**
 * One-off prompt dialog followed by the "keep this?" offer (specs/0079 W4). The follow-up's
 * labels are latched when it opens, so the dialog does not change while it animates out.
 */
export function CustomPromptFlow({ open, onClose, promptsApi, onFinished }: CustomPromptFlowProps) {
  const { state, saveOneOff, saveFollowup } = promptsApi;
  const [seed, setSeed] = useState<FollowupSeed | null>(null);
  // `seed` is kept after the follow-up closes so its labels hold while it animates out.
  const [followup, setFollowup] = useState(false);

  const finish = () => {
    onClose();
    onFinished?.();
  };

  return (
    <>
      <OneOffPromptDialog
        open={open && !followup}
        onOpenChange={(next) => {
          if (!next) onClose();
        }}
        initialBody={state?.custom_body ?? ''}
        initialExtract={state?.custom_extract_action_items ?? true}
        onSubmit={async (body, extract) => {
          await saveOneOff(body, extract);
          setSeed(seedFrom(body, state));
          setFollowup(true);
        }}
      />
      <SaveOneOffDialog
        open={open && followup}
        onOpenChange={() => {}}
        hasSeries={seed?.hasSeries ?? false}
        defaultName={seed?.defaultName ?? ''}
        onSave={async (opts) => {
          await saveFollowup(opts);
          setFollowup(false);
          finish();
        }}
        onSkip={() => {
          setFollowup(false);
          finish();
        }}
      />
    </>
  );
}
