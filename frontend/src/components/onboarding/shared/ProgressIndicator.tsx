import React from 'react';
import { Check, Lock, Download, CheckCircle2, BrainCircuit } from 'lucide-react';

interface ProgressIndicatorProps {
  current: number;
  total: number;
  onStepClick?: (step: number) => void;
}

const stepIcons = [
  Lock,         // 1. Welcome
  BrainCircuit, // 2. Setup Overview
  Download,     // 3. Download Progress
  // Step 4 (Permissions) doesn't need icon - auto-skipped on non-macOS
];

export function ProgressIndicator({ current, total, onStepClick }: ProgressIndicatorProps) {
  const visibleSteps = Array.from({ length: total }, (_, i) => i + 1);

  return (
    <div className="mb-8">
      <div className="flex items-center justify-center gap-2">
        {visibleSteps.map((step, index) => {
          const isActive = step === current;
          const isCompleted = step < current;
          const isClickable = isCompleted && onStepClick;
          const StepIcon = stepIcons[step - 1] || CheckCircle2;

          return (
            <React.Fragment key={step}>
              {/* Position marker — engraved bar with its step icon above it. */}
              <button
                onClick={() => isClickable && onStepClick(step)}
                disabled={!isClickable}
                aria-label={`Step ${step} of ${total}${isCompleted ? ' — completed' : isActive ? ' — current' : ''}`}
                aria-current={isActive ? 'step' : undefined}
                className={`flex flex-col items-center gap-1.5 ${
                  isClickable ? 'cursor-pointer hover:opacity-80' : 'cursor-default'
                }`}
              >
                {isCompleted ? (
                  <Check className="h-3.5 w-3.5 text-success transition-colors duration-300" />
                ) : (
                  <StepIcon
                    className={`h-3.5 w-3.5 transition-colors duration-300 ${
                      isActive ? 'text-brand' : 'text-muted-foreground'
                    }`}
                  />
                )}
                <span
                  className={`h-2 w-6 rounded-[1px] transition-all duration-300 ${
                    isCompleted
                      ? 'bg-success'
                      : isActive
                        ? 'bg-brand shadow-[0_0_0_1px_hsl(var(--brand)/0.25),0_0_8px_-1px_hsl(var(--brand)/0.55)]'
                        : 'bg-border'
                  }`}
                />
              </button>

              {/* Connector Line */}
              {index < visibleSteps.length - 1 && (
                <div
                  className={`h-0.5 w-6 transition-all duration-300 ${
                    isCompleted ? 'bg-success' : 'bg-border'
                  }`}
                />
              )}
            </React.Fragment>
          );
        })}
      </div>
    </div>
  );
}
