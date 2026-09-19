'use client';

import { useCallback, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { toast } from 'sonner';
import type { ModelConfig } from '@/components/ModelSettingsModal';
import { isOllamaNotInstalledError } from '@/lib/utils';
import type { BuiltInModelInfo } from '@/lib/builtin-ai';

/**
 * The provider-readiness checks that run before a summary is generated.
 *
 * Local providers can be *configured* without being *usable* — the built-in model may still
 * be downloading or corrupted, Ollama may not be installed or may have no models pulled — and
 * the generation call itself gives a poor error in those cases. These guards catch each state
 * and say what to do about it, then open model settings.
 *
 * Extracted verbatim from `SummaryGeneratorButtonGroup` (specs/0064 W3) so the toolbar could
 * be rebuilt around it without touching the riskiest code on the page. Every message and
 * duration is unchanged: they are the only feedback a user gets when a provider is misset.
 */
export function useSummaryGenerationGuards({
  modelConfig,
  customPrompt,
  onGenerateSummary,
  onNeedsModelSettings,
}: {
  modelConfig: ModelConfig;
  customPrompt: string;
  onGenerateSummary: (customPrompt: string) => Promise<void>;
  /** Open the model-settings dialog — every failure path below ends here. */
  onNeedsModelSettings: () => void;
}): { isCheckingModels: boolean; generate: () => Promise<void> } {
  const [isCheckingModels, setIsCheckingModels] = useState(false);

  const checkBuiltInAIModelsAndGenerate = useCallback(async () => {
    setIsCheckingModels(true);
    try {
      const selectedModel = modelConfig.model;

      // Check if specific model is configured
      if (!selectedModel) {
        toast.error('No built-in AI model selected', {
          description: 'Please select a model in settings',
          duration: 5000,
        });
        onNeedsModelSettings();
        return;
      }

      // Check model readiness (with filesystem refresh)
      const isReady = await invoke<boolean>('builtin_ai_is_model_ready', {
        modelName: selectedModel,
        refresh: true,
      });

      if (isReady) {
        // Model is available, proceed with generation
        onGenerateSummary(customPrompt);
        return;
      }

      // Model not ready - check detailed status
      const modelInfo = await invoke<BuiltInModelInfo | null>('builtin_ai_get_model_info', {
        modelName: selectedModel,
      });

      if (!modelInfo) {
        toast.error('Model not found', {
          description: `Could not find information for model: ${selectedModel}`,
          duration: 5000,
        });
        onNeedsModelSettings();
        return;
      }

      // Handle different model states
      const status = modelInfo.status;

      if (status.type === 'downloading') {
        toast.info('Model download in progress', {
          description: `${selectedModel} is downloading (${status.progress}%). Please wait until download completes.`,
          duration: 5000,
        });
        return;
      }

      if (status.type === 'not_downloaded') {
        toast.error('Model not downloaded', {
          description: `${selectedModel} needs to be downloaded before use. Opening model settings...`,
          duration: 5000,
        });
        onNeedsModelSettings();
        return;
      }

      if (status.type === 'corrupted') {
        toast.error('Model file corrupted', {
          description: `${selectedModel} file is corrupted. Please delete and re-download.`,
          duration: 7000,
        });
        onNeedsModelSettings();
        return;
      }

      if (status.type === 'error') {
        toast.error('Model error', {
          description: status.Error || 'An error occurred with the model',
          duration: 5000,
        });
        onNeedsModelSettings();
        return;
      }

      // Fallback
      toast.error('Model not available', {
        description: 'The selected model is not ready for use',
        duration: 5000,
      });
      onNeedsModelSettings();
    } catch (error) {
      console.error('Error checking built-in AI models:', error);
      toast.error('Failed to check model status', {
        description: error instanceof Error ? error.message : String(error),
        duration: 5000,
      });
    } finally {
      setIsCheckingModels(false);
    }
  }, [modelConfig.model, customPrompt, onGenerateSummary, onNeedsModelSettings]);

  const generate = useCallback(async () => {
    // Handle built-in AI provider
    if (modelConfig.provider === 'builtin-ai') {
      await checkBuiltInAIModelsAndGenerate();
      return;
    }

    // Only check for Ollama provider
    if (modelConfig.provider !== 'ollama') {
      onGenerateSummary(customPrompt);
      return;
    }

    setIsCheckingModels(true);
    try {
      const endpoint = modelConfig.ollamaEndpoint || null;
      const models = (await invoke('get_ollama_models', { endpoint })) as unknown[];

      if (!models || models.length === 0) {
        // No models available, show message and open settings
        toast.error(
          'No Ollama models found. Please download gemma2:2b from Model Settings.',
          { duration: 5000 },
        );
        onNeedsModelSettings();
        return;
      }

      // Models are available, proceed with generation
      onGenerateSummary(customPrompt);
    } catch (error) {
      console.error('Error checking Ollama models:', error);
      const errorMessage = error instanceof Error ? error.message : String(error);

      if (isOllamaNotInstalledError(errorMessage)) {
        // Ollama is not installed - show specific message with download link
        toast.error('Ollama is not installed', {
          description: 'Please download and install Ollama to use local models.',
          duration: 7000,
          action: {
            label: 'Download',
            onClick: () =>
              invoke('open_external_url', { url: 'https://ollama.com/download' }),
          },
        });
      } else {
        // Other error - generic message
        toast.error(
          'Failed to check Ollama models. Please check if Ollama is running and download a model.',
          { duration: 5000 },
        );
      }
      onNeedsModelSettings();
    } finally {
      setIsCheckingModels(false);
    }
  }, [
    modelConfig.provider,
    modelConfig.ollamaEndpoint,
    customPrompt,
    onGenerateSummary,
    onNeedsModelSettings,
    checkBuiltInAIModelsAndGenerate,
  ]);

  return { isCheckingModels, generate };
}
