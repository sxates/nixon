import React, { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Button } from '@/components/ui/button';
import { OnboardingContainer } from '../OnboardingContainer';
import { useOnboarding } from '@/contexts/OnboardingContext';
import { getParakeetSizeLabel, getSummaryModelSizeLabel } from '@/lib/onboarding-summary-model';

// specs/0061 W1 Task 1 — the disk-space check backed by the Rust
// `get_models_disk_check` command (frontend/src-tauri/src/onboarding_disk.rs).
interface DiskCheck {
  free_bytes: number;
  required_bytes: number;
  ok: boolean;
  models_dir: string;
}

function formatBytes(bytes: number): string {
  return `${(bytes / 1024 ** 3).toFixed(1)} GiB`;
}

export function SetupOverviewStep() {
  const { goNext } = useOnboarding();
  const [isMac, setIsMac] = useState(false);
  const [summaryModel, setSummaryModel] = useState('');
  const [disk, setDisk] = useState<DiskCheck | null>(null);

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

  useEffect(() => {
    let cancelled = false;
    const checkDisk = async () => {
      try {
        const model = await invoke<string>('builtin_ai_get_recommended_model');
        if (cancelled) return;
        setSummaryModel(model);
        const result = await invoke<DiskCheck>('get_models_disk_check', { summaryModel: model });
        if (cancelled) return;
        setDisk(result);
      } catch (_e) {
        // Best-effort: leave the step usable even if the check fails.
      }
    };
    checkDisk();
    return () => {
      cancelled = true;
    };
  }, []);

  const needsMoreSpace = Boolean(disk && !disk.ok);

  const steps = [
    {
      number: 1,
      title: 'Download Transcription Engine',
      size: getParakeetSizeLabel(),
      description: 'On-device speech recognition (Parakeet). Nothing leaves your Mac.',
    },
    {
      number: 2,
      title: 'Download Summarization Engine',
      size: getSummaryModelSizeLabel(summaryModel),
      description:
        'Prefer OpenAI, Claude, or Ollama? You can switch providers later in Settings › Summary.',
    },
  ];

  const handleContinue = goNext;

  return (
    <OnboardingContainer
      title="Setup Overview"
      description="Nixon requires that you download the Transcription & Summarization AI models for the software to work."
      step={2}
      totalSteps={isMac ? 5 : 4}
    >
      <div className="flex flex-col items-center space-y-10">
        {/* Steps Card */}
        <div className="w-full max-w-md bg-card rounded-[3px] border border-border p-4">
          <div className="space-y-4" data-testid="setup-overview-steps">
            {steps.map((step) => (
              <div key={step.number} className="flex items-start gap-4 p-1">
                <div className="flex-1 ml-1">
                  <h3 className="font-medium text-foreground">
                    Step {step.number} : {step.title}
                  </h3>
                  <p className="text-sm text-muted-foreground">{step.description}</p>
                  <p className="text-xs text-muted-foreground">{step.size}</p>
                </div>
              </div>
            ))}
          </div>
        </div>

        {/* CTA Section */}
        <div className="w-full max-w-xs space-y-4">
          {needsMoreSpace && disk && (
            <div className="flex items-center gap-2 rounded-[3px] border border-border bg-card p-3">
              <span className="h-2 w-2 flex-shrink-0 rounded-full bg-record" />
              <span className="text-sm text-foreground">
                Not enough free space: {formatBytes(disk.free_bytes)} available, about{' '}
                {formatBytes(disk.required_bytes)} needed. Free some space or download anyway.
              </span>
            </div>
          )}
          <Button
            onClick={handleContinue}
            className="w-full h-11 bg-brand hover:bg-brand/90 text-brand-foreground"
          >
            {needsMoreSpace ? 'Download anyway' : "Let's Go"}
          </Button>
        </div>
      </div>
    </OnboardingContainer>
  );
}
