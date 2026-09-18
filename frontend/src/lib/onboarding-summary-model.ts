interface OnboardingSummaryModelStatusInput {
  selectedModel: string;
  recommendedModel: string;
  selectedModelReady: boolean;
}

interface OnboardingSummaryModelStatus {
  selectedSummaryModel: string;
  summaryModelDownloaded: boolean;
}

export const SUMMARY_MODEL_SIZES_MB: Record<string, number> = {
  'qwen3.5:2b': 1221,
  'qwen3.5:4b': 2614,
  'gemma3:1b': 1019,
  'gemma3:4b': 2374,
};

// specs/0061 W1 — Parakeet (transcription) model size, shown next to the
// summarization model size on the Setup Overview onboarding step.
export const PARAKEET_MODEL_SIZE_MB = 670;

export const getParakeetSizeLabel = (): string => '~670 MB';

export function resolveOnboardingSummaryModelStatus({
  selectedModel,
  recommendedModel,
  selectedModelReady,
}: OnboardingSummaryModelStatusInput): OnboardingSummaryModelStatus {
  const selectedSummaryModel = selectedModel || recommendedModel;

  return {
    selectedSummaryModel,
    summaryModelDownloaded: Boolean(selectedSummaryModel && selectedModelReady),
  };
}

export function getSummaryModelSizeMb(model: string): number {
  return SUMMARY_MODEL_SIZES_MB[model] ?? 0;
}

export function getDownloadTotalMb(totalMb: number | null | undefined, model: string): number {
  return totalMb || getSummaryModelSizeMb(model);
}

export function formatSummaryModelSizeLabelFromMb(sizeMb: number): string {
  if (sizeMb === 0) {
    return '';
  }

  if (sizeMb >= 1024) {
    return `~${(sizeMb / 1024).toFixed(1)} GiB`;
  }

  return `~${sizeMb} MiB`;
}

export function getSummaryModelSizeLabel(model: string): string {
  return formatSummaryModelSizeLabelFromMb(getSummaryModelSizeMb(model));
}
