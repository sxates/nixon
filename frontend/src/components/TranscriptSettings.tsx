import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from './ui/select';
import { Input } from './ui/input';
import { Button } from './ui/button';
import {
    SettingsGroup,
    SettingsRow,
    SettingsSection,
} from './ui/settings';
import { ResolvedSettingRow } from './ResolvedSettingRow';
import { Eye, EyeOff, Lock, Unlock } from 'lucide-react';

/** Mirrors `config.rs::DEFAULT_PARAKEET_MODEL`; the engine+model Nixon ships with. */
const DEFAULT_PARAKEET_MODEL = 'parakeet-tdt-0.6b-v3-int8';
import { ModelManager } from './WhisperModelManager';
import { ParakeetModelManager } from './ParakeetModelManager';


export interface TranscriptModelProps {
    provider: 'localWhisper' | 'parakeet' | 'deepgram' | 'elevenLabs' | 'groq' | 'openai';
    model: string;
    /**
     * Write-only key field (spec 0030 WS2): a masked hint ("••••1234") for a
     * stored key, or a replacement the user typed. Raw keys are never
     * returned by the backend.
     */
    apiKey?: string | null;
    /** From the backend getter: whether a key is stored for `provider`. */
    apiKeyConfigured?: boolean;
    /** From the backend getter: masked display hint for the stored key. */
    apiKeyMasked?: string | null;
}

export interface TranscriptSettingsProps {
    transcriptModelConfig: TranscriptModelProps;
    setTranscriptModelConfig: (config: TranscriptModelProps) => void;
    onModelSelect?: () => void;
}

export function TranscriptSettings({ transcriptModelConfig, setTranscriptModelConfig, onModelSelect }: TranscriptSettingsProps) {
    const [apiKey, setApiKey] = useState<string | null>(transcriptModelConfig.apiKey || null);
    const [showApiKey, setShowApiKey] = useState<boolean>(false);
    const [isApiKeyLocked, setIsApiKeyLocked] = useState<boolean>(true);
    const [isLockButtonVibrating, setIsLockButtonVibrating] = useState<boolean>(false);
    const [uiProvider, setUiProvider] = useState<TranscriptModelProps['provider']>(transcriptModelConfig.provider);

    // specs/0067 W2 — the engine picker is an expert control. Parakeet on its default model
    // is what Nixon ships and what the WER comparison backed (17.6% vs 20.6% against the
    // same corpus, and the engine built for the live path), so a user running that sees one
    // row saying so. Anything else — Whisper, a cloud provider, a hand-picked Parakeet
    // model — keeps its controls whether or not advanced options are on.
    const isDefaultEngine =
        transcriptModelConfig.provider === 'parakeet' &&
        transcriptModelConfig.model === DEFAULT_PARAKEET_MODEL;


    // Sync uiProvider when backend config changes (e.g., after model selection or initial load)
    useEffect(() => {
        setUiProvider(transcriptModelConfig.provider);
    }, [transcriptModelConfig.provider]);

    useEffect(() => {
        if (transcriptModelConfig.provider === 'localWhisper' || transcriptModelConfig.provider === 'parakeet') {
            setApiKey(null);
        }
    }, [transcriptModelConfig.provider]);

    const fetchApiKey = async (provider: string) => {
        try {
            // Configured/masked status only — raw keys never cross IPC (0030 WS2)
            const status = await invoke('api_get_transcript_api_key', { provider }) as
                { configured: boolean; masked?: string | null };
            setApiKey(status.configured ? (status.masked ?? '••••') : '');
        } catch (err) {
            console.error('Error fetching API key status:', err);
            setApiKey(null);
        }
    };
    const modelOptions = {
        localWhisper: [], // Model selection handled by ModelManager component
        parakeet: [], // Model selection handled by ParakeetModelManager component
        deepgram: ['nova-2-phonecall'],
        elevenLabs: ['eleven_multilingual_v2'],
        groq: ['llama-3.3-70b-versatile'],
        openai: ['gpt-4o'],
    };
    const requiresApiKey = transcriptModelConfig.provider === 'deepgram' || transcriptModelConfig.provider === 'elevenLabs' || transcriptModelConfig.provider === 'openai' || transcriptModelConfig.provider === 'groq';

    const handleInputClick = () => {
        if (isApiKeyLocked) {
            setIsLockButtonVibrating(true);
            setTimeout(() => setIsLockButtonVibrating(false), 500);
        }
    };

    const handleWhisperModelSelect = (modelName: string) => {
        // Always update config when model is selected, regardless of current provider
        // This ensures the model is set when user switches back
        setTranscriptModelConfig({
            ...transcriptModelConfig,
            provider: 'localWhisper', // Ensure provider is set correctly
            model: modelName
        });
        // Close modal after selection
        if (onModelSelect) {
            onModelSelect();
        }
    };

    const handleParakeetModelSelect = (modelName: string) => {
        // Always update config when model is selected, regardless of current provider
        // This ensures the model is set when user switches back
        setTranscriptModelConfig({
            ...transcriptModelConfig,
            provider: 'parakeet', // Ensure provider is set correctly
            model: modelName
        });
        // Close modal after selection
        if (onModelSelect) {
            onModelSelect();
        }
    };

    return (
        <div className="space-y-8">
            <SettingsSection
                title="Transcript model"
                description="Which engine turns meeting audio into text. Every option runs entirely on this Mac."
            >
                <ResolvedSettingRow
                    label="Engine"
                    description="Parakeet, running on this Mac. It keeps up with live audio and was the most accurate of the on-device engines on real meeting recordings."
                    value="Parakeet"
                    locked={!isDefaultEngine}
                >
                <SettingsGroup>
                    <SettingsRow
                        label="Engine"
                        htmlFor="transcript-provider"
                        description="Parakeet is fastest and accurate enough for live transcripts; Whisper is slower but more accurate on hard audio."
                        control={
                            <div className="flex w-64 gap-2">
                                <Select
                                    value={uiProvider}
                                    onValueChange={(value) => {
                                        const provider = value as TranscriptModelProps['provider'];
                                        setUiProvider(provider);
                                        if (provider !== 'localWhisper' && provider !== 'parakeet') {
                                            fetchApiKey(provider);
                                        }
                                    }}
                                >
                                    <SelectTrigger
                                        id="transcript-provider"
                                        className="focus:border-brand focus:ring-1 focus:ring-ring"
                                    >
                                        <SelectValue placeholder="Select provider" />
                                    </SelectTrigger>
                                    <SelectContent>
                                        <SelectItem value="parakeet">Parakeet (recommended)</SelectItem>
                                        <SelectItem value="localWhisper">Local Whisper</SelectItem>
                                    </SelectContent>
                                </Select>

                                {uiProvider !== 'localWhisper' && uiProvider !== 'parakeet' && (
                                    <Select
                                        value={transcriptModelConfig.model}
                                        onValueChange={(value) => {
                                            const model = value as TranscriptModelProps['model'];
                                            setTranscriptModelConfig({ ...transcriptModelConfig, provider: uiProvider, model });
                                        }}
                                    >
                                        <SelectTrigger className="focus:border-brand focus:ring-1 focus:ring-ring">
                                            <SelectValue placeholder="Select model" />
                                        </SelectTrigger>
                                        <SelectContent>
                                            {modelOptions[uiProvider].map((model) => (
                                                <SelectItem key={model} value={model}>{model}</SelectItem>
                                            ))}
                                        </SelectContent>
                                    </Select>
                                )}
                            </div>
                        }
                    />

                    {requiresApiKey && (
                        <SettingsRow
                            label="API key"
                            htmlFor="transcript-api-key"
                            description="Stored in the macOS Keychain. Unlock to replace it."
                            control={
                                <div className="relative w-64">
                                    <Input
                                        id="transcript-api-key"
                                        type={showApiKey ? 'text' : 'password'}
                                        className={`pr-24 focus:border-brand focus:ring-1 focus:ring-ring ${isApiKeyLocked ? 'cursor-not-allowed bg-muted' : ''}`}
                                        value={apiKey || ''}
                                        onChange={(e) => setApiKey(e.target.value)}
                                        disabled={isApiKeyLocked}
                                        onClick={handleInputClick}
                                        placeholder="Enter your API key"
                                    />
                                    <div className="absolute inset-y-0 right-0 flex items-center pr-1">
                                        <Button
                                            type="button"
                                            variant="ghost"
                                            size="icon"
                                            onClick={() => setIsApiKeyLocked(!isApiKeyLocked)}
                                            className={`transition-colors duration-200 ${isLockButtonVibrating ? 'text-destructive' : ''}`}
                                            title={isApiKeyLocked ? 'Unlock to edit' : 'Lock to prevent editing'}
                                        >
                                            {isApiKeyLocked ? <Lock className="h-4 w-4" /> : <Unlock className="h-4 w-4" />}
                                        </Button>
                                        <Button
                                            type="button"
                                            variant="ghost"
                                            size="icon"
                                            onClick={() => setShowApiKey(!showApiKey)}
                                            title={showApiKey ? 'Hide API key' : 'Show API key'}
                                        >
                                            {showApiKey ? <EyeOff className="h-4 w-4" /> : <Eye className="h-4 w-4" />}
                                        </Button>
                                    </div>
                                </div>
                            }
                        />
                    )}
                </SettingsGroup>
                {uiProvider === 'localWhisper' && (
                    <SettingsSection
                        title="Whisper models"
                        description="Download a model to use it. Larger models are more accurate and slower."
                    >
                        <SettingsGroup className="py-4">
                            <ModelManager
                                selectedModel={transcriptModelConfig.provider === 'localWhisper' ? transcriptModelConfig.model : undefined}
                                onModelSelect={handleWhisperModelSelect}
                                autoSave={true}
                            />
                        </SettingsGroup>
                    </SettingsSection>
                )}

                {uiProvider === 'parakeet' && (
                    <SettingsSection
                        title="Parakeet models"
                        description="Download a model to use it. Runs in real time on Apple Silicon."
                    >
                        <SettingsGroup className="py-4">
                            <ParakeetModelManager
                                selectedModel={transcriptModelConfig.provider === 'parakeet' ? transcriptModelConfig.model : undefined}
                                onModelSelect={handleParakeetModelSelect}
                                autoSave={true}
                            />
                        </SettingsGroup>
                    </SettingsSection>
                )}
                </ResolvedSettingRow>
            </SettingsSection>

        </div>
    );
}
