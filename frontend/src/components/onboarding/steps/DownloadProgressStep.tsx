import React, { useEffect, useState, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { safeListen } from '@/lib/safe-listen';
import { Button } from '@/components/ui/button';
import { Callout } from '@/components/ui/callout';
import { Progress } from '@/components/ui/progress';
import { Spinner } from '@/components/ui/spinner';
import { StatusDot } from '@/components/ui/status-dot';
import { OnboardingContainer } from '../OnboardingContainer';
import { useOnboarding } from '@/contexts/OnboardingContext';
import { toast } from 'sonner';
import { motion, AnimatePresence } from 'framer-motion';
import {
  getSummaryModelSizeLabel,
  getSummaryModelSizeMb,
  getParakeetSizeLabel,
  PARAKEET_MODEL_SIZE_MB,
} from '@/lib/onboarding-summary-model';

const PARAKEET_MODEL = 'parakeet-tdt-0.6b-v3-int8';

type DownloadStatus = 'waiting' | 'downloading' | 'completed' | 'error';

interface DownloadState {
  status: DownloadStatus;
  progress: number;
  downloadedMb: number;
  totalMb: number;
  speedMbps: number;
  error?: string;
}

export function DownloadProgressStep() {
  const {
    goNext,
    selectedSummaryModel,
    recommendedSummaryModel,
    parakeetDownloaded,
    setParakeetDownloaded,
    summaryModelDownloaded,
    setSummaryModelDownloaded,
    startBackgroundDownloads,
  } = useOnboarding();

  const [isMac, setIsMac] = useState(false);

  const [parakeetState, setParakeetState] = useState<DownloadState>({
    status: parakeetDownloaded ? 'completed' : 'waiting',
    progress: parakeetDownloaded ? 100 : 0,
    downloadedMb: 0,
    totalMb: PARAKEET_MODEL_SIZE_MB,
    speedMbps: 0,
  });

  const [summaryState, setSummaryState] = useState<DownloadState>({
    status: summaryModelDownloaded ? 'completed' : 'waiting',
    progress: summaryModelDownloaded ? 100 : 0,
    downloadedMb: 0,
    totalMb: 0,
    speedMbps: 0,
  });

  const parakeetDownloadStartedRef = useRef(false);
  const summaryDownloadStartedRef = useRef(false);
  const retryingRef = useRef(false);
  const retryingSummaryRef = useRef(false);

  // Retry download handler
  const handleRetryDownload = async () => {
    // Prevent multiple simultaneous retries
    if (retryingRef.current) {
      console.log('[DownloadProgressStep] Retry already in progress, ignoring');
      return;
    }

    console.log('[DownloadProgressStep] Retrying Parakeet download');
    retryingRef.current = true;

    // Reset error state
    setParakeetState((prev) => ({
      ...prev,
      status: 'waiting',
      error: undefined,
      progress: 0,
      downloadedMb: 0,
      speedMbps: 0,
    }));

    try {
      await invoke('parakeet_retry_download', { modelName: PARAKEET_MODEL });
      // Progress events will update state
    } catch (error) {
      console.error('[DownloadProgressStep] Retry failed:', error);
      setParakeetState((prev) => ({
        ...prev,
        status: 'error',
        error: error instanceof Error ? error.message : 'Retry failed',
      }));

      toast.error('Download retry failed', {
        description: 'Please check your connection and try again.',
      });
    } finally {
      // Allow retry again after 2 seconds
      setTimeout(() => {
        retryingRef.current = false;
      }, 2000);
    }
  };

  // Retry summary download handler
  const handleRetrySummaryDownload = async () => {
    // Prevent multiple simultaneous retries
    if (retryingSummaryRef.current) {
      console.log('[DownloadProgressStep] Summary retry already in progress, ignoring');
      return;
    }

    console.log('[DownloadProgressStep] Retrying summary model download');
    retryingSummaryRef.current = true;

    // Reset error state
    setSummaryState((prev) => ({
      ...prev,
      status: 'downloading',
      error: undefined,
      progress: 0,
      downloadedMb: 0,
      totalMb: getSummaryModelSizeMb(selectedSummaryModel || recommendedSummaryModel),
      speedMbps: 0,
    }));

    try {
      // Call download command directly (no retry command exists for built-in AI)
      const modelName = selectedSummaryModel;
      if (!modelName) {
        throw new Error('Summary model recommendation is not ready yet');
      }
      await invoke('builtin_ai_download_model', { modelName });
    } catch (error) {
      console.error('[DownloadProgressStep] Summary retry failed:', error);
      setSummaryState((prev) => ({
        ...prev,
        status: 'error',
        error: error instanceof Error ? error.message : 'Retry failed',
      }));

      toast.error('Summary model download retry failed', {
        description: 'Please check your connection and try again.',
      });
    } finally {
      // Allow retry again after 2 seconds
      setTimeout(() => {
        retryingSummaryRef.current = false;
      }, 2000);
    }
  };

  // Detect platform on mount
  useEffect(() => {
    const checkPlatform = async () => {
      try {
        const { platform } = await import('@tauri-apps/plugin-os');
        setIsMac(platform() === 'macos');
      } catch (_e) {
        setIsMac(navigator.userAgent.includes('Mac'));
      }
    };

    checkPlatform();
  }, []);

  // Start the required transcription model immediately; summary readiness must not block it.
  useEffect(() => {
    if (parakeetDownloadStartedRef.current) return;
    parakeetDownloadStartedRef.current = true;

    if (!parakeetDownloaded) {
      setParakeetState((prev) => ({ ...prev, status: 'downloading' }));
    }

    startBackgroundDownloads({
      includeParakeet: true,
      includeSummary: false,
    }).catch((error) => {
      console.error('Failed to start Parakeet download:', error);
      if (!parakeetDownloaded) {
        setParakeetState((prev) => ({ ...prev, status: 'error', error: String(error) }));
      }
    });
  }, []);

  // Start the selected summary model only after the backend recommendation is known.
  useEffect(() => {
    if (summaryDownloadStartedRef.current) return;
    if (!selectedSummaryModel) return;
    summaryDownloadStartedRef.current = true;

    startSummaryDownload();
  }, [selectedSummaryModel]);

  // Listen to Parakeet download progress
  useEffect(() => {
    const disposeProgress = safeListen<{
      modelName: string;
      progress: number;
      downloaded_mb?: number;
      total_mb?: number;
      speed_mbps?: number;
      status?: string;
    }>('parakeet-model-download-progress', (event) => {
      const { modelName, progress, downloaded_mb, total_mb, speed_mbps, status } = event.payload;
      if (modelName === PARAKEET_MODEL) {
        setParakeetState((prev) => ({
          ...prev,
          status: status === 'completed' ? 'completed' : 'downloading',
          progress,
          downloadedMb: downloaded_mb ?? prev.downloadedMb,
          totalMb: total_mb ?? prev.totalMb,
          speedMbps: speed_mbps ?? prev.speedMbps,
        }));

        if (status === 'completed' || progress >= 100) {
          setParakeetDownloaded(true);
        }
      }
    });

    const disposeComplete = safeListen<{ modelName: string }>(
      'parakeet-model-download-complete',
      (event) => {
        if (event.payload.modelName === PARAKEET_MODEL) {
          setParakeetState((prev) => ({ ...prev, status: 'completed', progress: 100 }));
          setParakeetDownloaded(true);
        }
      }
    );

    const disposeError = safeListen<{ modelName: string; error: string }>(
      'parakeet-model-download-error',
      (event) => {
        if (event.payload.modelName === PARAKEET_MODEL) {
          setParakeetState((prev) => ({
            ...prev,
            status: 'error',
            error: event.payload.error,
          }));
        }
      }
    );

    return () => {
      disposeProgress();
      disposeComplete();
      disposeError();
    };
  }, []);

  // Listen to Summary Model download progress (always downloading for builtin-ai)
  useEffect(() => {
    return safeListen<{
      model: string;
      progress: number;
      downloaded_mb?: number;
      total_mb?: number;
      speed_mbps?: number;
      status: string;
      error?: string;
    }>('builtin-ai-download-progress', (event) => {
      const { model, progress, downloaded_mb, total_mb, speed_mbps, status, error } = event.payload;
      if (selectedSummaryModel && model === selectedSummaryModel) {
        setSummaryState((prev) => ({
          ...prev,
          status: status === 'completed'
            ? 'completed'
            : status === 'error'
            ? 'error'
            : 'downloading',
          progress,
          downloadedMb: downloaded_mb ?? prev.downloadedMb,
          totalMb: (total_mb ?? prev.totalMb) || getSummaryModelSizeMb(model),
          speedMbps: speed_mbps ?? prev.speedMbps,
          error: status === 'error' ? error : undefined,
        }));

        if (status === 'completed' || progress >= 100) {
          setSummaryModelDownloaded(true);
        }
      }
    });
  }, [selectedSummaryModel]);

  useEffect(() => {
    const modelForSize = selectedSummaryModel || recommendedSummaryModel;
    if (!modelForSize) return;

    setSummaryState((prev) => ({
      ...prev,
      status: summaryModelDownloaded
        ? 'completed'
        : prev.status === 'completed'
        ? 'waiting'
        : prev.status,
      progress: summaryModelDownloaded
        ? 100
        : prev.status === 'completed'
        ? 0
        : prev.progress,
      totalMb: prev.totalMb || getSummaryModelSizeMb(modelForSize),
    }));
  }, [selectedSummaryModel, recommendedSummaryModel, summaryModelDownloaded]);

  const startSummaryDownload = async () => {
    if (!summaryModelDownloaded && selectedSummaryModel) {
      try {
        setSummaryState((prev) => ({
          ...prev,
          status: 'downloading',
          totalMb: getSummaryModelSizeMb(selectedSummaryModel),
        }));
        await startBackgroundDownloads({
          includeParakeet: false,
          includeSummary: true,
          summaryModel: selectedSummaryModel,
        });
      } catch (error) {
        console.error('Failed to start summary model download:', error);
        setSummaryState((prev) => ({ ...prev, status: 'error', error: String(error) }));
      }
    }
  };

  const handleContinue = async () => {
    // Verify actual model availability (catches state drift)
    try {
      await invoke('parakeet_init');
      const actuallyAvailable = await invoke<boolean>('parakeet_has_available_models');

      if (actuallyAvailable && !parakeetDownloaded) {
        console.log('[DownloadProgressStep] Model available but state not updated');
        setParakeetDownloaded(true);
        setParakeetState((prev) => ({
          ...prev,
          status: 'completed',
          progress: 100,
        }));
      } else if (!actuallyAvailable && parakeetState.status === 'error') {
        toast.error('Transcription engine required', {
          description: 'Please retry the download before continuing.',
        });
        return;
      }
    } catch (error) {
      console.warn('[DownloadProgressStep] Failed to verify model:', error);
    }

    // macOS: goes to Permissions, then Calendar (completes onboarding there).
    // Non-macOS: skips Permissions and goes straight to Calendar (completes there).
    goNext();
  };

  const renderDownloadCard = (
    title: string,
    state: DownloadState,
    modelSize: string,
    sizeUnit = 'MB'
  ) => (
    <div className="rounded-lg border border-border bg-card p-5">
      <div className="mb-4 flex items-center justify-between">
        <div>
          <h3 className="font-medium text-foreground">{title}</h3>
          <p className="text-sm text-muted-foreground">{modelSize}</p>
        </div>
        <div className="flex items-center text-muted-foreground">
          {state.status === 'waiting' && (
            <span className="text-sm text-muted-foreground">Waiting...</span>
          )}
          {state.status === 'downloading' && <Spinner size="md" />}
          {state.status === 'completed' && <StatusDot tone="success" label="Downloaded" />}
          {state.status === 'error' && (
            <span className="text-sm text-destructive">Failed</span>
          )}
        </div>
      </div>

      {/* Progress Bar */}
      {(state.status === 'downloading' || state.status === 'completed') && (
        <div className="space-y-2">
          <Progress value={state.progress} aria-label={`${title} download progress`} />
          <div className="flex items-center justify-between text-sm">
            <span className="text-muted-foreground">
              {state.downloadedMb.toFixed(1)} {sizeUnit} / {state.totalMb.toFixed(1)} {sizeUnit}
            </span>
            <div className="flex items-center gap-2">
              {state.speedMbps > 0 && (
                <span className="text-muted-foreground">
                  {state.speedMbps.toFixed(1)} {sizeUnit}/s
                </span>
              )}
              <span className="font-semibold text-foreground">
                {Math.round(state.progress)}%
              </span>
            </div>
          </div>
        </div>
      )}

      {state.status === 'error' && state.error && (
        <Callout tone="error" className="mt-2">
          <p className="font-medium text-destructive">Download Error</p>
          <p className="mt-1 text-xs text-foreground">{state.error}</p>
          {(title === 'Transcription Engine' || title === 'Summary Engine') && (
            <Button
              variant="brand"
              className="mt-3 w-full"
              onClick={title === 'Transcription Engine' ? handleRetryDownload : handleRetrySummaryDownload}
            >
              Try Again
            </Button>
          )}
        </Callout>
      )}
    </div>
  );

  return (
    <OnboardingContainer
      title="Getting things ready"
      description="You can start using Nixon after downloading the Transcription Engine."
      step={3}
      totalSteps={isMac ? 5 : 4}
    >
      <div className="flex flex-col items-center space-y-6">
        {/* Download Cards */}
        <div className="w-full max-w-lg space-y-4">
          {renderDownloadCard(
            'Transcription Engine',
            parakeetState,
            getParakeetSizeLabel()
          )}

          {renderDownloadCard(
            'Summary Engine',
            summaryState,
            getSummaryModelSizeLabel(selectedSummaryModel || recommendedSummaryModel),
            'MiB'
          )}
        </div>

        {/* Info Message - Only show when Parakeet is downloaded */}
        <AnimatePresence>
          {parakeetDownloaded && !summaryModelDownloaded && (
            <motion.div
              initial={{ opacity: 0 }}
              animate={{ opacity: 1 }}
              exit={{ opacity: 0 }}
              transition={{ duration: 0.14, ease: 'easeOut' }}
              className="w-full max-w-lg"
            >
              <Callout tone="muted" title="You can continue while this finishes">
                Download will continue in the background.
              </Callout>
            </motion.div>
          )}
        </AnimatePresence>

        {/* Continue Button */}
        <div className="w-full max-w-xs">
          <Button
            variant="brand"
            size="lg"
            className="w-full"
            onClick={handleContinue}
            disabled={!parakeetDownloaded}
            aria-busy={!parakeetDownloaded || undefined}
          >
            {!parakeetDownloaded ? <Spinner /> : 'Continue'}
          </Button>
        </div>
      </div>
    </OnboardingContainer>
  );
}
