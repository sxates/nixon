import { ModelConfig } from "@/components/ModelSettingsModal";
import { PreferenceSettings } from "@/components/PreferenceSettings";
import { DeviceSelection } from "@/components/DeviceSelection";
import { TranscriptSettings } from "@/components/TranscriptSettings";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Callout } from "@/components/ui/callout";
import { DeckIcon } from "@/components/ui/deck-icon";
import { IconButton } from "@/components/ui/icon-button";
import { X } from "@/components/ui/icons";
import { Label } from "@/components/ui/label";
import { OptionCard, OptionCardGroup } from "@/components/ui/option-card";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import { toast } from "sonner";
import { useConfig } from "@/contexts/ConfigContext";
import { useRecordingState } from "@/contexts/RecordingStateContext";
import { splitErrorAlert } from "@/lib/error-alert";

type modalType = "modelSettings" | "deviceSettings" | "modelSelector" | "errorAlert" | "chunkDropWarning";

/**
 * SettingsModals Component
 *
 * All settings modals consolidated into a single component.
 * Uses ConfigContext and RecordingStateContext internally - no prop drilling needed!
 */

interface SettingsModalsProps {
  modals: {
    modelSettings: boolean;
    deviceSettings: boolean;
    modelSelector: boolean;
    errorAlert: boolean;
    chunkDropWarning: boolean;
  };
  messages: {
    errorAlert: string;
    chunkDropWarning: string;
    modelSelector: string;
  };
  onClose: (name: modalType) => void;
}

export function SettingsModals({
  modals,
  messages,
  onClose,
}: SettingsModalsProps) {
  // Contexts
  const {
    modelConfig,
    setModelConfig,
    models,
    modelOptions,
    error,
    selectedDevices,
    setSelectedDevices,
    transcriptModelConfig,
    setTranscriptModelConfig,
    showConfidenceIndicator,
    toggleConfidenceIndicator,
  } = useConfig();

  const { isRecording } = useRecordingState();
  const errorAlert = splitErrorAlert(messages.errorAlert);

  return <>
    {/* Legacy Settings Modal */}
    {modals.modelSettings && (
      <div className={"fixed inset-0 bg-scrim flex items-center justify-center z-50 p-4"}>
        <div className="bg-card rounded-lg shadow-xl max-w-4xl w-full max-h-[90vh] overflow-hidden flex flex-col">
          {/* Header */}
          <div className="flex justify-between items-center p-6 border-b border-border">
            <h3 className="text-title font-semibold text-foreground">Preferences</h3>
            <IconButton variant="ghost" label="Close" icon={<DeckIcon icon={X} size={16} />} onClick={() => onClose("modelSettings")} />
          </div>

          {/* Content - Scrollable */}
          <div className="flex-1 overflow-y-auto p-6 space-y-8">
            {/* General Preferences Section */}
            <PreferenceSettings />

            {/* Divider */}
            <div className="border-t border-border pt-8">
              <h4 className="text-heading font-semibold text-foreground mb-4">AI Model Configuration</h4>
              <div className="space-y-4">
                <div>
                  <Label htmlFor="legacy-summary-provider" className="mb-1 block">
                    Summarization Model
                  </Label>
                  <div className="flex space-x-2">
                    <Select
                      value={modelConfig.provider}
                      onValueChange={(value) => {
                        const provider = value as ModelConfig['provider'];
                        setModelConfig({
                          ...modelConfig,
                          provider,
                          model: modelOptions[provider][0]
                        });
                      }}
                    >
                      <SelectTrigger id="legacy-summary-provider" className="w-40 bg-card">
                        <SelectValue />
                      </SelectTrigger>
                      <SelectContent>
                        <SelectItem value="builtin-ai">Built-in AI</SelectItem>
                        <SelectItem value="claude">Claude</SelectItem>
                        <SelectItem value="groq">Groq</SelectItem>
                        <SelectItem value="ollama">Ollama</SelectItem>
                        <SelectItem value="openrouter">OpenRouter</SelectItem>
                        <SelectItem value="openai">OpenAI</SelectItem>
                      </SelectContent>
                    </Select>

                    <Select
                      value={modelConfig.model}
                      onValueChange={(value) => setModelConfig((prev: ModelConfig) => ({ ...prev, model: value }))}
                    >
                      <SelectTrigger aria-label="Model" className="flex-1 bg-card">
                        <SelectValue />
                      </SelectTrigger>
                      <SelectContent>
                        {modelOptions[modelConfig.provider].map((model: string) => (
                          <SelectItem key={model} value={model}>
                            {model}
                          </SelectItem>
                        ))}
                      </SelectContent>
                    </Select>
                  </div>
                </div>
                {modelConfig.provider === 'ollama' && (
                  <div>
                    <h4 className="text-heading font-semibold mb-4">Available Ollama Models</h4>
                    {error && (
                      <Callout tone="error" className="mb-4">
                        {error}
                      </Callout>
                    )}
                    <OptionCardGroup
                      aria-label="Available Ollama models"
                      value={modelConfig.model}
                      onValueChange={(name) => setModelConfig((prev: ModelConfig) => ({ ...prev, model: name }))}
                      className="grid gap-2 max-h-[400px] overflow-y-auto pr-2"
                    >
                      {models.map((model) => (
                        <OptionCard
                          key={model.id}
                          value={model.name}
                          title={model.name}
                          description={<>Size: {model.size} · Modified: {model.modified}</>}
                        />
                      ))}
                    </OptionCardGroup>
                  </div>
                )}
              </div>
            </div>
          </div>

          {/* Footer */}
          <div className="border-t border-border p-6 flex justify-end">
            <Button variant="brand" onClick={() => onClose('modelSettings')}>
              Done
            </Button>
          </div>
        </div>
      </div>
    )}

    {/* Device Settings Modal */}
    {modals.deviceSettings && (
      <div className={"fixed inset-0 bg-scrim flex items-center justify-center z-50"}>
        <div className="bg-card rounded-lg p-6 max-w-md w-full mx-4 shadow-xl">
          <div className="flex justify-between items-center mb-4">
            <h3 className="text-lg font-semibold text-foreground">Audio Device Settings</h3>
            <IconButton variant="ghost" label="Close" icon={<DeckIcon icon={X} size={16} />} onClick={() => onClose('deviceSettings')} />
          </div>

          <DeviceSelection
            selectedDevices={selectedDevices}
            onDeviceChange={setSelectedDevices}
            disabled={isRecording}
          />

          <div className="mt-6 flex justify-end">
            <Button
              variant="brand"
              onClick={() => {
                const micDevice = selectedDevices.micDevice || 'Default';
                const systemDevice = selectedDevices.systemDevice || 'Default';
                toast.success("Devices selected", {
                  description: `Microphone: ${micDevice}, System Audio: ${systemDevice}`
                });
                onClose('deviceSettings');
              }}
            >
              Done
            </Button>
          </div>
        </div>
      </div>
    )}

    {/* Model Selection Modal */}
    {modals.modelSelector && (
      <div className={"fixed inset-0 bg-scrim flex items-center justify-center z-50"}>
        <div className="bg-card rounded-lg max-w-4xl w-full mx-4 shadow-xl max-h-[90vh] flex flex-col">
          {/* Fixed Header */}
          <div className="flex justify-between items-center p-6 pb-4 border-b border-border">
            <h3 className="text-lg font-semibold text-foreground">
              {messages.modelSelector ? 'Speech Recognition Setup Required' : 'Transcription Model Settings'}
            </h3>
            <IconButton variant="ghost" label="Close" icon={<DeckIcon icon={X} size={16} />} onClick={() => onClose('modelSelector')} />
          </div>

          {/* Scrollable Content */}
          <div className="flex-1 overflow-y-auto p-6 pt-4">
            <TranscriptSettings
              transcriptModelConfig={transcriptModelConfig}
              setTranscriptModelConfig={setTranscriptModelConfig}
              onModelSelect={() => onClose('modelSelector')}
            />
          </div>

          {/* Fixed Footer */}
          <div className="p-6 pt-4 border-t border-border flex items-center justify-between">
            {/* Confidence Indicator Toggle */}
            <div className="flex items-center gap-3">
              <Switch
                checked={showConfidenceIndicator}
                onCheckedChange={toggleConfidenceIndicator}
                aria-label="Show Confidence Indicators"
              />
              <div>
                <p className="text-sm font-medium text-foreground">Show Confidence Indicators</p>
                <p className="text-xs text-muted-foreground">Display colored dots showing transcription confidence quality</p>
              </div>
            </div>

            <Button variant="outline" onClick={() => onClose('modelSelector')}>
              {messages.modelSelector ? 'Cancel' : 'Done'}
            </Button>
          </div>
        </div>
      </div>
    )}

    {/* Error Alert Modal */}
    {modals.errorAlert && (
      <div className={"fixed inset-0 bg-scrim flex items-center justify-center z-50"}>
        {/* An optional first line is the heading (a failed START names the device problem);
            the rest keeps its newlines, so the bullet lists render as bullets. Messages with
            no newline keep the historical heading. See lib/error-alert.ts. */}
        <Alert className="max-w-md mx-4 border-destructive-edge bg-card shadow-xl">
          <AlertTitle className="text-destructive">{errorAlert.title}</AlertTitle>
          <AlertDescription className="whitespace-pre-line text-destructive">
            {errorAlert.body}
            <Button
              variant="link"
              onClick={() => onClose('errorAlert')}
              className="ml-2 h-auto whitespace-normal p-0 text-destructive underline"
            >
              Dismiss
            </Button>
          </AlertDescription>
        </Alert>
      </div>
    )}

    {/* Chunk Drop Warning Modal */}
    {modals.chunkDropWarning && (
      <div className={"fixed inset-0 bg-scrim flex items-center justify-center z-50"}>
        <Alert className="max-w-lg mx-4 border-brand-edge bg-card shadow-xl">
          <AlertTitle className="text-brand">Transcription Performance Warning</AlertTitle>
          <AlertDescription className="text-brand">
            {messages.chunkDropWarning}
            <Button
              variant="link"
              onClick={() => onClose('chunkDropWarning')}
              className="ml-2 h-auto p-0 text-brand underline"
            >
              Dismiss
            </Button>
          </AlertDescription>
        </Alert>
      </div>
    )}
  </>
}
