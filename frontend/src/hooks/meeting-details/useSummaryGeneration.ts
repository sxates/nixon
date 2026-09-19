import { useState, useCallback, useEffect } from 'react';
import { Transcript, Summary, SummaryChunkStatus } from '@/types';
import { ModelConfig } from '@/components/ModelSettingsModal';
import { useSidebar } from '@/components/Sidebar/SidebarProvider';
import { invoke as invokeTauri } from '@tauri-apps/api/core';
import { fetchAllTranscripts as fetchAllTranscriptRows } from '@/lib/fetch-all-transcripts';
import { listen } from '@tauri-apps/api/event';
import { safeListen } from '@/lib/safe-listen';
import { toast } from 'sonner';
import { isOllamaNotInstalledError } from '@/lib/utils';
import { BuiltInModelInfo } from '@/lib/builtin-ai';
import { useConfig } from '@/contexts/ConfigContext';
import {
  needsTranscriptionBeforeSummary,
  retranscriptionProviderFor,
  SPARSE_TRANSCRIPT_SEGMENTS,
} from '@/lib/deferred-transcription';
import { resolveSummaryLanguage } from '@/lib/resolve-summary-language';

// 'speaker_refresh' (specs/0041 WS2): the backend is quietly regenerating the summary
// with freshly-diarized speaker names — the existing summary stays on screen.
type SummaryStatus = 'idle' | 'processing' | 'summarizing' | 'regenerating' | 'speaker_refresh' | 'completed' | 'error';

interface UseSummaryGenerationProps {
  meeting: any;
  transcripts: Transcript[];
  modelConfig: ModelConfig;
  isModelConfigLoading: boolean;
  selectedTemplate: string;
  onMeetingUpdated?: () => Promise<void>;
  updateMeetingTitle: (title: string) => void;
  setAiSummary: (summary: Summary | null) => void;
  onOpenModelSettings?: () => void;
  /** specs/0029 WS7.2: refresh the transcript panel after a deferred
   *  (transcribe-before-summarize) transcription lands new rows. */
  refetchTranscripts?: () => Promise<void>;
}

export function useSummaryGeneration({
  meeting,
  transcripts: _transcripts,
  modelConfig,
  isModelConfigLoading,
  selectedTemplate,
  onMeetingUpdated,
  updateMeetingTitle,
  setAiSummary,
  onOpenModelSettings,
  refetchTranscripts,
}: UseSummaryGenerationProps) {
  const [summaryStatus, setSummaryStatus] = useState<SummaryStatus>('idle');
  const [summaryError, setSummaryError] = useState<string | null>(null);

  const { startSummaryPolling, stopSummaryPolling } = useSidebar();
  // specs/0029 WS7.2: which local STT provider to use for deferred transcription.
  const { transcriptModelConfig } = useConfig();

  // Helper to get status message
  const getSummaryStatusMessage = useCallback((status: SummaryStatus) => {
    switch (status) {
      case 'processing':
        return 'Processing transcript...';
      case 'summarizing':
        return 'Generating summary...';
      case 'regenerating':
        return 'Regenerating summary...';
      case 'speaker_refresh':
        return 'Updating with speaker names…';
      case 'completed':
        return 'Summary completed';
      case 'error':
        return 'Error generating summary';
      default:
        return '';
    }
  }, []);

  // specs/0041 WS2: when offline diarization lands AFTER the auto-summary already ran,
  // the backend regenerates the (speakerless, pristine) summary itself and emits
  // `summary-refresh-started` the moment the regeneration actually kicks off — on the
  // immediate path (just before `diarization-complete`) AND on the deferred path, where
  // the refresh waits behind an in-flight summary run and starts minutes after
  // `diarization-complete` already fired with `summaryRefreshing: false`. Listening to
  // this single dedicated event (instead of the old `diarization-complete`
  // `summaryRefreshing` flag, which missed the deferred path) covers both. Reflect the
  // run quietly: keep the current summary on screen with an "Updating with speaker
  // names…" strip and reuse the same api_get_summary polling the manual flows use. The
  // backend resets the process row to PENDING before emitting, so the first poll can't
  // read a stale 'completed'. Double-start safe: startSummaryPolling replaces any
  // active poll for the meeting (one interval per meeting id), so a duplicate event
  // just restarts the same poll.
  useEffect(() => {
    const dispose = safeListen<{ meeting_id: string }>(
      'summary-refresh-started',
      (event) => {
        if (event.payload.meeting_id !== meeting.id) return;
        console.log('🔄 Backend is refreshing the summary with speaker names');
        setSummaryStatus('speaker_refresh');
        setSummaryError(null);

        startSummaryPolling(meeting.id, meeting.id, async (pollingResult) => {
          if (pollingResult.status === 'completed' && pollingResult.data?.markdown) {
            const chunkStatus = pollingResult.data.summary_status as SummaryChunkStatus | undefined;
            setAiSummary({ markdown: pollingResult.data.markdown, summary_status: chunkStatus } as any);
            setSummaryStatus('completed');
            return;
          }
          if (
            pollingResult.status === 'error' ||
            pollingResult.status === 'failed' ||
            pollingResult.status === 'cancelled'
          ) {
            // Quiet failure: the backend already restored the previous summary from
            // its backup — reload it and step out of the updating state. No toast;
            // the pre-refresh summary is still perfectly valid.
            console.warn('Speaker-name summary refresh did not complete:', pollingResult.error);
            try {
              const existing = await invokeTauri('api_get_summary', {
                meetingId: meeting.id,
              }) as any;
              if (existing?.data) {
                setAiSummary(existing.data);
                setSummaryStatus('completed');
                return;
              }
            } catch (error) {
              console.warn('Failed to reload summary after refresh failure:', error);
            }
            setSummaryStatus('idle');
          }
        });
      },
    );
    return () => {
      dispose();
    };
  }, [meeting.id, startSummaryPolling, setAiSummary]);

  // Unified summary processing logic
  const processSummary = useCallback(async ({
    transcriptText,
    transcriptTexts,
    customPrompt = '',
    isRegeneration = false,
    notesGrounded = false,
    background = false,
  }: {
    transcriptText: string;
    transcriptTexts?: string[];
    customPrompt?: string;
    isRegeneration?: boolean;
    // Notes-only / pre-transcript meetings (spec 0015): there's no transcript,
    // but the backend grounds the summary on the user's saved notes. When true we
    // allow an empty transcript through; `process_transcript_background` loads the
    // notes from the DB and routes the empty transcript into the single-pass
    // notes-grounding synthesis.
    notesGrounded?: boolean;
    // specs/0063 W3 Task 6b: true only for the auto-summary that fires by itself after a
    // recording stops. It registers as background work so it appears in the rail's Queue
    // and survives the user navigating away; a run the user asked for stays foreground,
    // because this view already shows it its own ChunkProgressDisplay.
    background?: boolean;
  }) => {
    setSummaryStatus(isRegeneration ? 'regenerating' : 'processing');
    setSummaryError(null);

    try {
      if (!transcriptText.trim() && !notesGrounded) {
        throw new Error('No transcript text available. Please add some text first.');
      }

      console.log('Processing transcript with template:', selectedTemplate);

      // Show toast notification for generation start
      toast.info(`${isRegeneration ? 'Regenerating' : 'Generating'} summary...`, {
        description: `Using ${modelConfig.provider}/${modelConfig.model}`,
        duration: 3000,
      });

      // Resolve explicit metadata override first; Auto detects the transcript
      // language. For a notes-grounded summary there's no transcript to detect
      // from, so pass null and let the backend fall back to the notes' language.
      const summaryLanguage = notesGrounded
        ? null
        : await resolveSummaryLanguage(
            meeting.id,
            transcriptTexts?.length ? transcriptTexts : [transcriptText]
          );

      // Process transcript and get process_id
      const result = await invokeTauri('api_process_transcript', {
        text: transcriptText,
        model: modelConfig.provider,
        modelName: modelConfig.model,
        meetingId: meeting.id,
        chunkSize: 40000,
        overlap: 1000,
        customPrompt: customPrompt,
        templateId: selectedTemplate,
        summaryLanguage,
        background,
      }) as any;

      const process_id = result.process_id;
      console.log('Process ID:', process_id);

      // Start global polling via context
      startSummaryPolling(meeting.id, process_id, async (pollingResult) => {
        console.log('Summary status:', pollingResult);

        // Handle cancellation
        if (pollingResult.status === 'cancelled') {
          console.log('Summary generation was cancelled');

          // Reload summary from database (backend has already restored from backup)
          try {
            const existingSummary = await invokeTauri('api_get_summary', {
              meetingId: meeting.id
            }) as any;

            if (existingSummary?.data) {
              console.log('Restored previous summary after cancellation');
              setAiSummary(existingSummary.data);
              setSummaryStatus('completed');
            } else {
              setSummaryStatus('idle');
            }
          } catch (error) {
            console.error('Failed to reload summary after cancellation:', error);
            setSummaryStatus('idle');
          }

          setSummaryError(null);
          return;
        }

        // Handle errors
        if (pollingResult.status === 'error' || pollingResult.status === 'failed') {
          console.error('Backend returned error:', pollingResult.error);
          const errorMessage = pollingResult.error || `Summary ${isRegeneration ? 'regeneration' : 'generation'} failed`;

          // If this was a regeneration, try to restore previous summary from database
          if (isRegeneration) {
            try {
              const existingSummary = await invokeTauri('api_get_summary', {
                meetingId: meeting.id
              }) as any;

              if (existingSummary?.data) {
                console.log('Restored previous summary after regeneration failure');
                setAiSummary(existingSummary.data);
                setSummaryStatus('completed');
                setSummaryError(null);

                // Show error toast with restoration message
                toast.error(`Failed to regenerate summary`, {
                  description: `${errorMessage}. Your previous summary has been restored.`,
                });

                return;
              }
            } catch (error) {
              console.error('Failed to reload summary after error:', error);
            }
          }

          // Continue with normal error handling if not regeneration or reload failed
          setSummaryError(errorMessage);
          setSummaryStatus('error');

          // Check if this is a "model is required" error
          const isModelRequiredError = errorMessage.includes('model is required') ||
            errorMessage.includes('"model":"required"') ||
            errorMessage.toLowerCase().includes('model') && errorMessage.toLowerCase().includes('required');

          // Show error toast
          toast.error(`Failed to ${isRegeneration ? 'regenerate' : 'generate'} summary`, {
            description: errorMessage.includes('Connection refused')
              ? 'Could not connect to LLM service. Please ensure Ollama or your configured LLM provider is running.'
              : errorMessage,
          });

          // Auto-open model settings modal if model is missing
          if (isModelRequiredError && onOpenModelSettings) {
            console.log('🔧 Model required error detected, opening model settings...');
            onOpenModelSettings();
          }

          return;
        }

        // Handle successful completion
        if (pollingResult.status === 'completed' && pollingResult.data) {
          console.log('Summary generation completed:', pollingResult.data);

          // Update meeting title if available — UNLESS the user owns the title: a meeting
          // that adopted a calendar event's identity (specs/0015 Join & Record, the event
          // name) OR one the user manually edited (specs/0024 WS6.1, `titleManuallySet`). The
          // backend skips persisting an LLM name in both cases, so don't transiently flip the
          // displayed title here either; an ad-hoc date-stamp title is still auto-named.
          const titleIsAuthoritative = !!meeting.calendarEventId || !!meeting.titleManuallySet;
          const meetingName = pollingResult.data.MeetingName || pollingResult.meetingName;
          if (meetingName && !titleIsAuthoritative) {
            updateMeetingTitle(meetingName);
          }

          // Check if backend returned markdown format (new flow)
          if (pollingResult.data.markdown) {
            console.log('Received markdown format from backend');
            // Carry the chunk-outcome accounting (specs/0028 `summary_status`) into the
            // summary object so SummaryPanel can flag a PARTIAL summary. Absent on
            // results from older backends — treated as complete.
            const chunkStatus = pollingResult.data.summary_status as SummaryChunkStatus | undefined;
            setAiSummary({ markdown: pollingResult.data.markdown, summary_status: chunkStatus } as any);
            setSummaryStatus('completed');

            if (chunkStatus && chunkStatus.complete === false) {
              // Some transcript chunks failed after retries — warn instead of
              // celebrating; the panel shows a persistent banner with the details.
              toast.warning('Partial summary generated', {
                description: `${chunkStatus.failed_chunks} of ${chunkStatus.total_chunks} transcript section${chunkStatus.total_chunks === 1 ? '' : 's'} could not be processed, so some content may be missing. Regenerating may recover it.`,
                duration: 8000,
              });
            } else {
              // Show success toast
              toast.success('Summary generated successfully!', {
                description: 'Your meeting summary is ready',
                duration: 4000,
              });
            }

            if (meetingName && onMeetingUpdated) {
              await onMeetingUpdated();
            }

            return;
          }

          // Legacy format handling
          const summarySections = Object.entries(pollingResult.data).filter(([key]) => key !== 'MeetingName');
          const allEmpty = summarySections.every(([, section]) => !(section as any).blocks || (section as any).blocks.length === 0);

          if (allEmpty) {
            console.error('Summary completed but all sections empty');
            setSummaryError('Summary generation completed but returned empty content.');
            setSummaryStatus('error');

            return;
          }

          // Remove MeetingName from data before formatting
          const { MeetingName: _MeetingName, ...summaryData } = pollingResult.data;

          // Format legacy summary data
          const formattedSummary: Summary = {};
          const sectionKeys = pollingResult.data._section_order || Object.keys(summaryData);

          for (const key of sectionKeys) {
            try {
              const section = summaryData[key];
              if (section && typeof section === 'object' && 'title' in section && 'blocks' in section) {
                const typedSection = section as { title?: string; blocks?: any[] };

                if (Array.isArray(typedSection.blocks)) {
                  formattedSummary[key] = {
                    title: typedSection.title || key,
                    blocks: typedSection.blocks.map((block: any) => ({
                      ...block,
                      color: 'default',
                      content: block?.content?.trim() || ''
                    }))
                  };
                } else {
                  formattedSummary[key] = {
                    title: typedSection.title || key,
                    blocks: []
                  };
                }
              }
            } catch (error) {
              console.warn(`Error processing section ${key}:`, error);
            }
          }

          setAiSummary(formattedSummary);
          setSummaryStatus('completed');

          // Show success toast
          toast.success('Summary generated successfully!', {
            description: 'Your meeting summary is ready',
            duration: 4000,
          });

          if (meetingName && onMeetingUpdated) {
            await onMeetingUpdated();
          }
        }
      });
    } catch (error) {
      console.error(`Failed to ${isRegeneration ? 'regenerate' : 'generate'} summary:`, error);
      const errorMessage = error instanceof Error ? error.message : 'Unknown error';
      setSummaryError(errorMessage);
      setSummaryStatus('error');
      // Note: We don't clear the summary here because the backend has already restored from backup

      toast.error(`Failed to ${isRegeneration ? 'regenerate' : 'generate'} summary`, {
        description: errorMessage,
      });

    }
  }, [
    meeting.id,
    meeting.created_at,
    meeting.calendarEventId,
    modelConfig,
    selectedTemplate,
    startSummaryPolling,
    setAiSummary,
    updateMeetingTitle,
    onMeetingUpdated,
  ]);

  // Helper function to fetch ALL transcripts for summary generation
  const fetchAllTranscripts = useCallback(async (meetingId: string): Promise<Transcript[]> => {
    try {
      return await fetchAllTranscriptRows(meetingId);
    } catch (error) {
      console.error('❌ Error fetching all transcripts:', error);
      toast.error('Failed to fetch transcripts for summary generation');
      return [];
    }
  }, []);

  // True when the meeting has non-empty saved notes — used to decide whether a
  // transcript-less meeting can still be summarized (notes-grounded). Mirrors the
  // abandoned-recording notes check in useRecordingStop.
  const meetingHasNotes = useCallback(async (meetingId: string): Promise<boolean> => {
    try {
      const notes = await invokeTauri<{ notesMarkdown: string | null; notesJson: string | null } | null>(
        'api_get_meeting_notes',
        { meetingId },
      );
      const json = notes?.notesJson;
      return (
        (!!notes?.notesMarkdown && notes.notesMarkdown.trim().length > 0) ||
        (!!json && json.trim().length > 0 && json.trim() !== '[]')
      );
    } catch (error) {
      console.warn('Could not check meeting notes for summary eligibility:', error);
      return false;
    }
  }, []);

  // specs/0029 WS7.2: transcribe a record-only meeting's audio before summarizing.
  // Reuses the offline retranscription engine (decode → VAD → STT → persist) and its
  // progress events; resolves true on `retranscription-complete`, false on error /
  // failure to start (the caller then falls back to the pre-existing behavior:
  // notes-grounded summary or a friendly bail-out). Progress is surfaced through one
  // updating toast so the user sees the transcription phase before the summary phase.
  const transcribeMeetingAudio = useCallback(
    async (meetingId: string, folderPath: string): Promise<boolean> => {
      const toastId = `deferred-transcription-${meetingId}`;
      toast.loading('Transcribing meeting audio…', {
        id: toastId,
        description: 'This meeting was recorded without live transcription.',
      });

      let resolveCompletion: (ok: boolean) => void = () => {};
      const completion = new Promise<boolean>((resolve) => {
        resolveCompletion = resolve;
      });

      // Register ALL listeners before starting so a fast completion can't be missed.
      const unlisteners: Array<() => void> = [];
      try {
        unlisteners.push(
          await listen<{ meeting_id: string; progress_percentage: number; message: string }>(
            'retranscription-progress',
            (event) => {
              if (event.payload.meeting_id !== meetingId) return;
              toast.loading(
                `Transcribing meeting audio… ${Math.round(event.payload.progress_percentage)}%`,
                { id: toastId, description: event.payload.message },
              );
            },
          ),
        );
        unlisteners.push(
          await listen<{ meeting_id: string }>('retranscription-complete', (event) => {
            if (event.payload.meeting_id === meetingId) resolveCompletion(true);
          }),
        );
        unlisteners.push(
          await listen<{ meeting_id: string; error: string }>('retranscription-error', (event) => {
            if (event.payload.meeting_id !== meetingId) return;
            toast.error('Could not transcribe the meeting audio', {
              description: event.payload.error,
            });
            resolveCompletion(false);
          }),
        );

        await invokeTauri('start_retranscription_command', {
          meetingId,
          meetingFolderPath: folderPath,
          language: null,
          model: null,
          provider: retranscriptionProviderFor(transcriptModelConfig?.provider),
        });

        const ok = await completion;
        if (ok) {
          toast.success('Transcript ready', {
            description: 'Meeting audio transcribed — generating the summary now.',
          });
        }
        return ok;
      } catch (error) {
        console.error('Failed to start deferred transcription:', error);
        toast.error('Could not transcribe the meeting audio', {
          description: error instanceof Error ? error.message : String(error),
        });
        return false;
      } finally {
        toast.dismiss(toastId);
        unlisteners.forEach((unlisten) => unlisten());
      }
    },
    [transcriptModelConfig?.provider],
  );

  const buildSummaryTranscriptPayload = useCallback((allTranscripts: Transcript[]) => {
    const formatTime = (seconds: number | undefined, fallbackTimestamp: string): string => {
      if (seconds === undefined) {
        return fallbackTimestamp;
      }
      const totalSecs = Math.floor(seconds);
      const mins = Math.floor(totalSecs / 60);
      const secs = totalSecs % 60;
      return `[${mins.toString().padStart(2, '0')}:${secs.toString().padStart(2, '0')}]`;
    };

    return {
      transcriptText: allTranscripts
        .map(t => `${formatTime(t.audio_start_time, t.timestamp)} ${t.text}`)
        .join('\n'),
      transcriptTexts: allTranscripts.map(t => t.text),
    };
  }, []);

  // Public API: Generate summary from transcripts
  const handleGenerateSummary = useCallback(async (
    customPrompt: string = '',
    opts: { background?: boolean } = {},
  ) => {
    // Check if model config is still loading
    if (isModelConfigLoading) {
      console.log('⏳ Model configuration is still loading, please wait...');
      toast.info('Loading model configuration, please wait...');
      return;
    }

    // CHANGE: Fetch ALL transcripts from database, not from pagination state
    console.log('📊 Fetching all transcripts for summary generation...');
    let allTranscripts = await fetchAllTranscripts(meeting.id);

    // specs/0029 WS7.2: a record-only meeting has audio on disk but no/sparse
    // transcript rows — transcribe first, then summarize. Only fires when audio is
    // POSITIVELY available; probe failures leave the pre-existing behavior intact.
    if (allTranscripts.length < SPARSE_TRANSCRIPT_SEGMENTS && meeting.folder_path) {
      let audioAvailable: boolean | null = null;
      try {
        audioAvailable = await invokeTauri<boolean>('api_meeting_audio_available', {
          meetingId: meeting.id,
        });
      } catch (error) {
        console.warn('Could not check audio availability before summary:', error);
      }
      if (needsTranscriptionBeforeSummary(allTranscripts.length, audioAvailable)) {
        console.log('🎙️ Record-only meeting — running deferred transcription before summary');
        const transcribed = await transcribeMeetingAudio(meeting.id, meeting.folder_path);
        if (transcribed) {
          allTranscripts = await fetchAllTranscripts(meeting.id);
          // Refresh the transcript panel so the new rows are visible alongside
          // the summary that's about to generate.
          await refetchTranscripts?.().catch((error) =>
            console.warn('Failed to refresh transcripts after deferred transcription:', error),
          );
        }
        // On failure, fall through: the notes-grounded path / friendly bail-out
        // below still applies exactly as before.
      }
    }

    // Notes-only / pre-transcript meetings (spec 0015): no transcript, but we can
    // still summarize by grounding on the user's saved notes. Only bail when
    // there's neither a transcript nor any notes.
    const notesGrounded = allTranscripts.length === 0;
    if (notesGrounded) {
      const hasNotes = await meetingHasNotes(meeting.id);
      if (!hasNotes) {
        toast.error('Add a transcript or some notes before generating a summary.');
        return;
      }
      console.log('📝 No transcript — generating a notes-grounded summary');
    } else {
      console.log(`✅ Proceeding with ${allTranscripts.length} transcripts`);
    }

    console.log('🚀 Starting summary generation with config:', {
      provider: modelConfig.provider,
      model: modelConfig.model,
      template: selectedTemplate
    });

    // Check if Ollama provider has models available
    if (modelConfig.provider === 'ollama') {
      try {
        const endpoint = modelConfig.ollamaEndpoint || null;
        const models = await invokeTauri('get_ollama_models', { endpoint }) as any[];

        if (!models || models.length === 0) {
          toast.error(
            'No Ollama models found. Please download gemma3:1b from Model Settings.',
            { duration: 5000 }
          );
          return;
        }
      } catch (error) {
        console.error('Error checking Ollama models:', error);
        const errorMessage = error instanceof Error ? error.message : String(error);

        if (isOllamaNotInstalledError(errorMessage)) {
          // Ollama is not installed - show specific message with download link
          toast.error(
            'Ollama is not installed',
            {
              description: 'Please download and install Ollama to use local models.',
              duration: 7000,
              action: {
                label: 'Download',
                onClick: () => invokeTauri('open_external_url', { url: 'https://ollama.com/download' })
              }
            }
          );
        } else {
          // Other error - generic message
          toast.error(
            'Failed to check Ollama models. Please ensure Ollama is running and download a model from Settings.',
            { duration: 5000 }
          );
        }
        return;
      }
    }

    // Check if built-in AI provider has models available
    if (modelConfig.provider === 'builtin-ai') {
      try {
        const selectedModel = modelConfig.model;

        if (!selectedModel) {
          toast.error('No built-in AI model selected', {
            description: 'Please select a model in settings',
            duration: 5000,
          });
          if (onOpenModelSettings) {
            onOpenModelSettings();
          }
          return;
        }

        // Check model readiness with filesystem refresh
        const isReady = await invokeTauri<boolean>('builtin_ai_is_model_ready', {
          modelName: selectedModel,
          refresh: true,
        });

        if (!isReady) {
          // Get detailed model status
          const modelInfo = await invokeTauri<BuiltInModelInfo | null>('builtin_ai_get_model_info', {
            modelName: selectedModel,
          });

          if (modelInfo) {
            const status = modelInfo.status;

            if (status.type === 'downloading') {
              toast.info('Model download in progress', {
                description: `${selectedModel} is downloading (${status.progress}%). Please wait until download completes.`,
                duration: 5000,
              });
              return;
            }

            if (status.type === 'not_downloaded') {
              toast.error('Built-in AI model not downloaded', {
                description: `${selectedModel} needs to be downloaded. Please download it in model settings.`,
                duration: 7000,
              });
              if (onOpenModelSettings) {
                onOpenModelSettings();
              }
              return;
            }

            if (status.type === 'corrupted' || status.type === 'error') {
              const errorDesc = status.type === 'error'
                ? status.Error || 'The model file has an error'
                : 'The model file is corrupted';
              toast.error('Built-in AI model not available', {
                description: `${errorDesc}. Please check model settings.`,
                duration: 7000,
              });
              if (onOpenModelSettings) {
                onOpenModelSettings();
              }
              return;
            }
          }

          // Fallback if we couldn't get model info
          toast.error('Built-in AI model not ready', {
            description: 'Please ensure the model is downloaded in settings',
            duration: 5000,
          });
          if (onOpenModelSettings) {
            onOpenModelSettings();
          }
          return;
        }

        // Model is ready, continue to backend call
      } catch (error) {
        console.error('Error validating built-in AI model:', error);
        toast.error('Failed to validate built-in AI model', {
          description: error instanceof Error ? error.message : String(error),
          duration: 5000,
        });
        return;
      }
    }

    const summaryPayload = notesGrounded
      ? { transcriptText: '', transcriptTexts: [] }
      : buildSummaryTranscriptPayload(allTranscripts);

    await processSummary({
      ...summaryPayload,
      customPrompt,
      notesGrounded,
      background: opts.background ?? false,
    });
  }, [meeting.id, meeting.folder_path, fetchAllTranscripts, meetingHasNotes, transcribeMeetingAudio, refetchTranscripts, buildSummaryTranscriptPayload, processSummary, modelConfig, isModelConfigLoading, selectedTemplate]);

  // Public API: Regenerate summary from the current saved transcript
  const handleRegenerateSummary = useCallback(async () => {
    const allTranscripts = await fetchAllTranscripts(meeting.id);

    const notesGrounded = allTranscripts.length === 0;
    if (notesGrounded && !(await meetingHasNotes(meeting.id))) {
      console.error('No transcript or notes available for regeneration');
      toast.error('No transcript or notes available for summary regeneration');
      return;
    }

    const regenPayload = notesGrounded
      ? { transcriptText: '', transcriptTexts: [] }
      : buildSummaryTranscriptPayload(allTranscripts);

    await processSummary({
      ...regenPayload,
      isRegeneration: true,
      notesGrounded,
    });
  }, [meeting.id, fetchAllTranscripts, meetingHasNotes, buildSummaryTranscriptPayload, processSummary]);

  // Public API: Stop ongoing summary generation
  const handleStopGeneration = useCallback(async () => {
    console.log('Stopping summary generation for meeting:', meeting.id);

    try {
      // Call backend to cancel the summary generation
      await invokeTauri('api_cancel_summary', {
        meetingId: meeting.id
      });
      console.log('✓ Backend cancellation request sent for meeting:', meeting.id);
    } catch (error) {
      console.error('Failed to cancel summary generation:', error);
      // Continue with frontend cleanup even if backend call fails
    }

    // Stop polling
    stopSummaryPolling(meeting.id);

    // Reset status to idle
    setSummaryStatus('idle');
    setSummaryError(null);

    // Show toast notification
    toast.info('Summary generation stopped', {
      description: 'You can generate a new summary anytime',
      duration: 3000,
    });
  }, [meeting.id, stopSummaryPolling]);

  return {
    summaryStatus,
    summaryError,
    handleGenerateSummary,
    handleRegenerateSummary,
    handleStopGeneration,
    getSummaryStatusMessage,
  };
}
