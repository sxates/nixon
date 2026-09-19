import { useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { toast } from 'sonner';
import { ModelConfig } from '@/components/ModelSettingsModal';

/**
 * Model-settings plumbing for meeting-details: the summary toolbar registers its
 * modal-open function here (via `handleRegisterModalOpen`), error handlers trigger it
 * (via `handleOpenModelSettings`), and `handleSaveModelConfig` persists a config to the
 * backend + broadcasts `model-config-updated` so ConfigContext stays in sync.
 */
export function useModelSettings() {
  // Ref to store the modal open function from the summary toolbar
  const openModelSettingsRef = useRef<(() => void) | null>(null);

  // Callback to register the modal open function
  const handleRegisterModalOpen = (openFn: () => void) => {
    console.log('📝 Registering modal open function in PageContent');
    openModelSettingsRef.current = openFn;
  };

  // Callback to trigger modal open (called from error handler)
  const handleOpenModelSettings = () => {
    console.log('🔔 Opening model settings from PageContent');
    if (openModelSettingsRef.current) {
      openModelSettingsRef.current();
    } else {
      console.warn('⚠️ Modal open function not yet registered');
    }
  };

  // Save model config to backend database and sync via event
  const handleSaveModelConfig = async (config?: ModelConfig) => {
    if (!config) return;
    try {
      await invoke('api_save_model_config', {
        provider: config.provider,
        model: config.model,
        whisperModel: config.whisperModel,
        apiKey: config.apiKey ?? null,
        ollamaEndpoint: config.ollamaEndpoint ?? null,
      });

      // Emit event so ConfigContext and other listeners stay in sync
      const { emit } = await import('@tauri-apps/api/event');
      await emit('model-config-updated', config);

      toast.success('Model settings saved successfully');
    } catch (error) {
      console.error('Failed to save model config:', error);
      toast.error('Failed to save model settings');
    }
  };

  return { handleRegisterModalOpen, handleOpenModelSettings, handleSaveModelConfig };
}
