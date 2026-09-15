'use client';

import { useState, useEffect, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { safeListen } from '@/lib/safe-listen';
import { toast } from 'sonner';
import { ModelConfig, ModelSettingsModal } from '@/components/ModelSettingsModal';
import { SummaryLanguageSettings } from '@/components/SummaryLanguageSettings';
import { Switch } from './ui/switch';
import { useConfig } from '@/contexts/ConfigContext';
import { SettingsGroup, SettingsRow, SettingsSection } from '@/components/ui/settings';

interface SummaryModelSettingsProps {
  refetchTrigger?: number; // Change this to trigger refetch
}

export function SummaryModelSettings({ refetchTrigger }: SummaryModelSettingsProps) {
  const [modelConfig, setModelConfig] = useState<ModelConfig>({
    provider: 'ollama',
    model: 'llama3.2:latest',
    whisperModel: 'large-v3',
    apiKey: null,
    ollamaEndpoint: null
  });

  const { isAutoSummary, toggleIsAutoSummary } = useConfig();

  // Reusable fetch function
  const fetchModelConfig = useCallback(async () => {
    try {
      const data = await invoke('api_get_model_config') as any;
      if (data && data.provider !== null) {
        // The backend returns configured/masked key status, never the raw key
        // (spec 0030 WS2). Surface the masked hint via the write-only apiKey field.
        if (data.provider !== 'ollama' && data.provider !== 'builtin-ai') {
          data.apiKey = data.apiKeyConfigured ? (data.apiKeyMasked ?? '••••') : null;
        }
        // Fetch Custom OpenAI config if that's the active provider
        if (data.provider === 'custom-openai') {
          try {
            const customConfig = (await invoke('api_get_custom_openai_config')) as any;
            if (customConfig) {
              data.customOpenAIDisplayName = customConfig.displayName || null;
              data.customOpenAIEndpoint = customConfig.endpoint || null;
              data.customOpenAIModel = customConfig.model || null;
              // Masked hint only (0030 WS2)
              data.customOpenAIApiKey = customConfig.apiKeyMasked || null;
              data.maxTokens = customConfig.maxTokens || null;
              data.temperature = customConfig.temperature || null;
              data.topP = customConfig.topP || null;
              // For custom-openai, model field should match customOpenAIModel
              data.model = customConfig.model || data.model;
            }
          } catch (err) {
            console.error('Failed to fetch custom OpenAI config:', err);
          }
        }
        setModelConfig(data);
      }
    } catch (error) {
      console.error('Failed to fetch model config:', error);
      toast.error('Failed to load model settings');
    }
  }, []);

  // Fetch on mount
  useEffect(() => {
    fetchModelConfig();
  }, [fetchModelConfig]);

  // Refetch when trigger changes (optional external control)
  useEffect(() => {
    if (refetchTrigger !== undefined && refetchTrigger > 0) {
      fetchModelConfig();
    }
  }, [refetchTrigger, fetchModelConfig]);

  // Listen for model config updates from other components
  useEffect(() => {
    return safeListen<ModelConfig>('model-config-updated', (event) => {
      console.log('SummaryModelSettings received model-config-updated event:', event.payload);
      setModelConfig(event.payload);
    });
  }, []);

  // Save handler
  const handleSaveModelConfig = async (config: ModelConfig) => {
    try {
      await invoke('api_save_model_config', {
        provider: config.provider,
        model: config.model,
        whisperModel: config.whisperModel,
        apiKey: config.apiKey,
        ollamaEndpoint: config.ollamaEndpoint,
      });

      setModelConfig(config);

      // Emit event to sync other components
      const { emit } = await import('@tauri-apps/api/event');
      await emit('model-config-updated', config);

      toast.success('Model settings saved successfully');
    } catch (error) {
      console.error('Error saving model config:', error);
      toast.error('Failed to save model settings');
    }
  };

  return (
    <div className="space-y-8">
      <SettingsSection
        title="Summary model"
        description="The AI model that writes your meeting summaries. A local model keeps everything on this Mac."
      >
        <SettingsGroup className="py-4">
          <ModelSettingsModal
            modelConfig={modelConfig}
            setModelConfig={setModelConfig}
            onSave={handleSaveModelConfig}
            skipInitialFetch={true}
          />
        </SettingsGroup>
      </SettingsSection>

      <SettingsSection
        title="When to summarize"
        description="Summaries can also be generated on demand from any meeting page."
      >
        <SettingsGroup>
          {/* Same ConfigContext state as the toggle in Recording settings (specs/0029
              WS7.3) — one source of truth, two surfaces. */}
          <SettingsRow
            label="Summarize automatically when a meeting ends"
            description="Also available in Settings → Recordings."
            control={
              <Switch
                checked={isAutoSummary}
                onCheckedChange={toggleIsAutoSummary}
                aria-label="Summarize automatically when a meeting ends"
              />
            }
          />
        </SettingsGroup>
      </SettingsSection>

      <SummaryLanguageSettings />
    </div>
  );
}
